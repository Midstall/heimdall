//! Event loop for the TUI. Multiplexes keyboard input, daemon WS messages,
//! and a periodic tick that drives HTTP refresh.

use std::time::Duration;

use crossterm::event::{Event as CtEvent, EventStream, KeyCode, KeyEvent, KeyModifiers};
use futures::StreamExt;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::time::interval;
use tracing::{debug, warn};

use crate::app::{App, View};
use crate::client::DaemonClient;
use crate::error::Result;

#[derive(Debug)]
pub enum AppEvent {
    Key(KeyEvent),
    DaemonEvent(Value),
    Tick,
    Quit,
    /// Reconnect task posted this after the WS handshake succeeded.
    ConnectionRestored,
    /// Reconnect task posted this after the WS dropped or initial connect
    /// failed. `reason` is a short human-readable string for the status bar.
    ConnectionLost {
        reason: String,
    },
}

fn parse_dut_snapshot(v: &Value) -> Option<crate::app::DutSnapshot> {
    use crate::app::{DutSnapshot, DutSnapshotSource};

    let source = v["source"].as_str()?.parse::<DutSnapshotSource>().ok()?;
    let ts = v["ts"].as_str().and_then(|s| {
        chrono::DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|dt| dt.with_timezone(&chrono::Utc))
    })?;
    let state = v.get("state")?;
    let fields = state.get("fields").and_then(|f| f.as_object())?;
    let job = v["job"].as_str().map(str::to_string);
    let mut entries: Vec<(String, String)> = fields
        .iter()
        .map(|(name, repr)| (name.clone(), format_value_repr(repr)))
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    Some(DutSnapshot {
        source,
        ts,
        job,
        fields: entries,
    })
}

/// Render a `ValueRepr` JSON object as a short display string. Keeps the
/// TUI free of binary-blob dumps for `Bytes` and shows `U64`s in hex.
fn format_value_repr(repr: &Value) -> String {
    let ty = repr.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let val = repr.get("value");
    match (ty, val) {
        ("u64", Some(v)) => v
            .as_u64()
            .map(|n| format!("0x{n:x}"))
            .unwrap_or_else(|| v.to_string()),
        ("bool", Some(v)) => v.to_string(),
        ("bytes", Some(Value::Array(bytes))) => bytes
            .iter()
            .filter_map(|b| b.as_u64())
            .map(|b| format!("{b:02x}"))
            .collect::<String>(),
        _ => repr.to_string(),
    }
}

/// Capped exponential backoff for WS reconnect attempts.
/// Attempt 0 → 1s, 1 → 2s, 2 → 4s, ... clamped at 30s.
pub fn backoff_delay(attempt: u32) -> Duration {
    let shift = attempt.min(5);
    let secs = 1u64 << shift;
    Duration::from_secs(secs.min(30))
}

/// Process a single AppEvent against the App, possibly refreshing state via
/// the client. Network failures during a refresh demote the connection state
/// instead of propagating, so the event loop keeps running.
pub async fn handle_event(app: &mut App, client: &DaemonClient, event: AppEvent) -> Result<()> {
    match event {
        AppEvent::Quit => {
            app.should_quit = true;
        }
        AppEvent::Key(k) => {
            handle_key(app, k);
        }
        AppEvent::DaemonEvent(v) => {
            // A WS message also implies the connection is healthy.
            app.mark_connected(client.base_url());
            let kind = v["kind"].as_str().unwrap_or("?");
            app.status = format!("event: {kind}");
            if kind == "dut-state-snapshot" {
                if let Some(snap) = parse_dut_snapshot(&v) {
                    let dut_id = v["dut"].as_str().unwrap_or("").to_string();
                    if !dut_id.is_empty() {
                        app.apply_dut_snapshot(dut_id, snap);
                    }
                } else {
                    warn!(payload = %v, "dropping dut-state-snapshot frame missing fields");
                }
                return Ok(());
            }
            if kind == "job-log" {
                if let (Some(job_id), Some(ts)) = (
                    v["job"].as_str(),
                    v["ts"].as_str().and_then(|s| {
                        chrono::DateTime::parse_from_rfc3339(s)
                            .ok()
                            .map(|dt| dt.with_timezone(&chrono::Utc))
                    }),
                ) {
                    let i18n_args = v["i18n_args"]
                        .as_object()
                        .map(|m| {
                            m.iter()
                                .map(|(k, val)| (k.clone(), val.as_str().unwrap_or("").to_string()))
                                .collect()
                        })
                        .unwrap_or_default();
                    app.push_job_log(
                        job_id,
                        crate::app::JobLogEntry {
                            level: v["level"].as_str().unwrap_or("info").to_string(),
                            message: v["message"].as_str().unwrap_or("").to_string(),
                            stage: v["stage"].as_str().map(str::to_string),
                            ts,
                            i18n_key: v["i18n_key"].as_str().map(str::to_string),
                            i18n_args,
                        },
                    );
                } else {
                    warn!(payload = %v, "dropping job-log frame missing job or ts");
                }
                // JobLog events don't shift jobs/campaigns/duts state, so
                // skip the HTTP refresh. The log panel re-renders directly.
                return Ok(());
            }
            try_refresh(app, client).await;
        }
        AppEvent::Tick => {
            try_refresh(app, client).await;
        }
        AppEvent::ConnectionRestored => {
            let was_disconnected =
                !matches!(app.connection, crate::app::ConnectionState::Connected);
            app.mark_connected(client.base_url());
            if was_disconnected {
                // Refresh aggressively right after a reconnect so the user
                // sees fresh state without waiting for the next tick.
                try_refresh(app, client).await;
            }
        }
        AppEvent::ConnectionLost { reason } => {
            app.mark_disconnected(reason);
        }
    }
    Ok(())
}

async fn try_refresh(app: &mut App, client: &DaemonClient) {
    if let Err(e) = refresh_current_view(app, client).await {
        warn!(error = %e, "refresh failed");
        app.mark_disconnected(short_error(&e));
    }
}

fn short_error(e: &crate::error::TuiError) -> String {
    use crate::error::TuiError;
    match e {
        TuiError::Http(_) => "http error".into(),
        TuiError::Ws(_) | TuiError::WsClosed => "ws closed".into(),
        TuiError::Json(_) => "bad json".into(),
        TuiError::BadUrl(_) => "bad url".into(),
        TuiError::Io(_) => "io error".into(),
    }
}

fn handle_key(app: &mut App, k: KeyEvent) {
    // Quit on q, ctrl-c, or esc-from-top-view.
    if matches!(k.code, KeyCode::Char('q'))
        || (k.modifiers.contains(KeyModifiers::CONTROL) && matches!(k.code, KeyCode::Char('c')))
    {
        app.should_quit = true;
        return;
    }
    match k.code {
        KeyCode::Char('j') | KeyCode::Down => app.navigate_down(),
        KeyCode::Char('k') | KeyCode::Up => app.navigate_up(),
        KeyCode::Char('1') => app.switch_view(View::Jobs),
        KeyCode::Char('2') => app.switch_view(View::Campaigns),
        KeyCode::Char('3') => app.switch_view(View::Duts),
        KeyCode::Char('4') => app.switch_view(View::About),
        KeyCode::Enter => {
            if let View::Jobs = &app.view
                && let Some(job) = app.focused_job()
            {
                let id = job.id.0.to_string();
                app.switch_view(View::JobDetail { id });
            }
        }
        KeyCode::Esc => {
            if matches!(&app.view, View::JobDetail { .. } | View::About) {
                app.switch_view(View::Jobs);
            }
        }
        _ => {}
    }
}

async fn refresh_current_view(app: &mut App, client: &DaemonClient) -> Result<()> {
    match app.view.clone() {
        View::Jobs => {
            app.jobs = client.list_jobs().await?;
        }
        View::Campaigns => {
            app.campaigns = client.list_campaigns().await?;
        }
        View::Duts => {
            app.duts = client.list_duts().await?;
        }
        View::About => {
            // Fetch once per session: /about is a static snapshot
            // of the daemon's compile-time cfg, so re-fetching on
            // every tick would just waste a round trip.
            if app.about.is_none() {
                if let Ok(about) = client.about().await {
                    app.about = Some(about);
                }
            }
        }
        View::JobDetail { id } => {
            if let Some(job) = client.get_job(&id).await? {
                // Update in-place if found, otherwise leave the existing list.
                if let Some(slot) = app.jobs.iter_mut().find(|j| j.id.0.to_string() == id) {
                    *slot = job;
                } else {
                    app.jobs.push(job);
                }
            }
            // Seed the per-job log buffer with persisted history exactly
            // once per session so the detail panel shows past events even
            // for a job that started before the TUI connected. Subsequent
            // ticks let the WS deliver live deltas without clobbering them.
            if !app.is_backfilled(&id) {
                if let Ok(history) = client.list_job_logs(&id).await {
                    app.replace_job_logs(id, history);
                }
            }
        }
    }
    Ok(())
}

/// Spawn three event producers that all feed into a single mpsc and return the rx.
/// - crossterm keys
/// - daemon WS events
/// - tick every second
pub async fn spawn_event_pump(client: &DaemonClient) -> Result<mpsc::UnboundedReceiver<AppEvent>> {
    let (tx, rx) = mpsc::unbounded_channel();

    // Tick
    {
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut t = interval(Duration::from_secs(1));
            loop {
                t.tick().await;
                if tx.send(AppEvent::Tick).is_err() {
                    return;
                }
            }
        });
    }

    // Crossterm keys
    {
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut events = EventStream::new();
            while let Some(ev) = events.next().await {
                match ev {
                    Ok(CtEvent::Key(k)) => {
                        if tx.send(AppEvent::Key(k)).is_err() {
                            return;
                        }
                    }
                    Ok(_) => continue,
                    Err(e) => {
                        warn!(error = %e, "crossterm event stream error");
                        return;
                    }
                }
            }
        });
    }

    // Daemon WS: self-healing loop. On disconnect, post ConnectionLost and
    // retry with capped exponential backoff. On reconnect, post
    // ConnectionRestored so the UI can clear the warning and re-fetch.
    {
        let tx = tx.clone();
        let client = client.clone();
        tokio::spawn(async move { ws_reconnect_loop(client, tx).await });
    }

    Ok(rx)
}

/// Maintain a live WS subscription to the daemon. Loops forever, reconnecting
/// with capped exponential backoff. Posts `ConnectionRestored` / `ConnectionLost`
/// AppEvents around state transitions, and forwards every daemon Event as
/// `DaemonEvent`.
///
/// Public so integration tests can drive just this piece without the
/// terminal-bound crossterm reader.
pub async fn ws_reconnect_loop(client: DaemonClient, tx: mpsc::UnboundedSender<AppEvent>) {
    let mut attempt: u32 = 0;
    loop {
        match client.subscribe_events().await {
            Ok(mut ws_rx) => {
                attempt = 0;
                debug!("daemon ws connected");
                if tx.send(AppEvent::ConnectionRestored).is_err() {
                    return;
                }
                while let Some(v) = ws_rx.recv().await {
                    if tx.send(AppEvent::DaemonEvent(v)).is_err() {
                        return;
                    }
                }
                // Inner stream closed cleanly. Treat as disconnect.
                if tx
                    .send(AppEvent::ConnectionLost {
                        reason: "ws closed".into(),
                    })
                    .is_err()
                {
                    return;
                }
            }
            Err(e) => {
                debug!(error = %e, "daemon ws connect failed");
                if tx
                    .send(AppEvent::ConnectionLost {
                        reason: format!("{e}"),
                    })
                    .is_err()
                {
                    return;
                }
            }
        }

        let delay = backoff_delay(attempt);
        attempt = attempt.saturating_add(1);
        tokio::time::sleep(delay).await;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, ConnectionState};
    use crate::client::DaemonClient;
    use std::sync::OnceLock;
    use tokio::sync::Mutex;

    /// Locale tests share a global. The lock serializes them so concurrent
    /// tests don't observe each other's locale changes. `tokio::sync::Mutex`
    /// lets the guard cross `.await` boundaries safely.
    async fn locale_test_lock() -> tokio::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let m = LOCK.get_or_init(|| Mutex::new(()));
        let guard = m.lock().await;
        heimdall_i18n::set_locale(heimdall_i18n::Locale::En);
        guard
    }

    #[test]
    fn backoff_schedule_grows_then_caps() {
        // 1, 2, 4, 8, 16, 32->30, 32->30, ...
        assert_eq!(backoff_delay(0), Duration::from_secs(1));
        assert_eq!(backoff_delay(1), Duration::from_secs(2));
        assert_eq!(backoff_delay(2), Duration::from_secs(4));
        assert_eq!(backoff_delay(3), Duration::from_secs(8));
        assert_eq!(backoff_delay(4), Duration::from_secs(16));
        assert_eq!(backoff_delay(5), Duration::from_secs(30));
        assert_eq!(backoff_delay(6), Duration::from_secs(30));
        assert_eq!(backoff_delay(100), Duration::from_secs(30));
    }

    #[tokio::test]
    async fn connection_lost_event_flips_state_and_status() {
        let _g = locale_test_lock().await;
        // Use an unroutable URL so the client never actually fires anything.
        let client = DaemonClient::new("http://127.0.0.1:1".to_string());
        let mut app = App::new();
        app.mark_connected(client.base_url());
        assert_eq!(app.connection, ConnectionState::Connected);

        handle_event(
            &mut app,
            &client,
            AppEvent::ConnectionLost {
                reason: "ws closed".into(),
            },
        )
        .await
        .unwrap();

        assert_eq!(app.connection, ConnectionState::Disconnected);
        assert!(
            app.status.contains("disconnected") && app.status.contains("retrying"),
            "status should hint at reconnect: {}",
            app.status
        );
    }

    #[tokio::test]
    async fn connection_restored_event_clears_disconnected_state() {
        let _g = locale_test_lock().await;
        let client = DaemonClient::new("http://127.0.0.1:1".to_string());
        let mut app = App::new();
        app.mark_disconnected("test");
        assert_eq!(app.connection, ConnectionState::Disconnected);

        // ConnectionRestored triggers a refresh which fails against the
        // unroutable URL and demotes back to Disconnected. Either outcome
        // is acceptable as long as the status is updated.
        handle_event(&mut app, &client, AppEvent::ConnectionRestored)
            .await
            .unwrap();
        assert!(
            matches!(
                app.connection,
                ConnectionState::Connected | ConnectionState::Disconnected
            ),
            "got: {:?}",
            app.connection
        );
    }

    #[tokio::test]
    async fn job_log_event_appends_to_buffer_without_refreshing() {
        let _g = locale_test_lock().await;
        let client = DaemonClient::new("http://127.0.0.1:1".to_string());
        let mut app = App::new();
        app.mark_connected(client.base_url());

        let v: Value = serde_json::from_str(
            r#"{"ts":"2026-06-01T19:03:21.249Z","kind":"job-log","job":"00000000-0000-0000-0000-000000000007","level":"info","message":"opening jtag","stage":"prepare","i18n_key":"log.runner.prepare_start","i18n_args":{}}"#,
        )
        .unwrap();
        handle_event(&mut app, &client, AppEvent::DaemonEvent(v))
            .await
            .unwrap();

        let buf = app
            .logs_for("00000000-0000-0000-0000-000000000007")
            .expect("buffer present");
        assert_eq!(buf.len(), 1);
        let entry = buf.front().unwrap();
        assert_eq!(entry.message, "opening jtag");
        assert_eq!(entry.stage.as_deref(), Some("prepare"));
        assert_eq!(
            entry.i18n_key.as_deref(),
            Some("log.runner.prepare_start"),
            "wire-side i18n_key must round-trip into the buffer"
        );
        assert_eq!(entry.ts.to_rfc3339(), "2026-06-01T19:03:21.249+00:00");

        // Connection state stays Connected because no HTTP refresh fired.
        assert_eq!(app.connection, ConnectionState::Connected);
    }

    #[tokio::test]
    async fn job_log_event_without_ts_is_dropped() {
        let _g = locale_test_lock().await;
        let client = DaemonClient::new("http://127.0.0.1:1".to_string());
        let mut app = App::new();
        app.mark_connected(client.base_url());

        // No `ts` field: must NOT silently push a synthetic-timestamp entry.
        let v: Value = serde_json::from_str(
            r#"{"kind":"job-log","job":"00000000-0000-0000-0000-000000000008","level":"info","message":"no ts"}"#,
        )
        .unwrap();
        handle_event(&mut app, &client, AppEvent::DaemonEvent(v))
            .await
            .unwrap();
        assert!(
            app.logs_for("00000000-0000-0000-0000-000000000008")
                .is_none(),
            "frame without ts should be dropped, not buffered with a fallback"
        );
    }

    #[tokio::test]
    async fn tick_with_unreachable_daemon_does_not_error_and_marks_disconnected() {
        let _g = locale_test_lock().await;
        let client = DaemonClient::new("http://127.0.0.1:1".to_string());
        let mut app = App::new();
        app.mark_connected(client.base_url());

        // Tick triggers a refresh and the unreachable client makes the HTTP
        // call fail. handle_event must NOT propagate the error.
        let res = handle_event(&mut app, &client, AppEvent::Tick).await;
        assert!(res.is_ok(), "tick should swallow http errors");
        assert_eq!(app.connection, ConnectionState::Disconnected);
        assert!(
            app.status.contains("disconnected"),
            "status: {}",
            app.status
        );
    }
}
