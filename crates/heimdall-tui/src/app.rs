//! App state: which view is active, cached state from the daemon, focus.

use std::collections::{HashMap, HashSet, VecDeque};

use chrono::{DateTime, Utc};
use heimdall_daemon::{Campaign, Job};
use serde::Deserialize;

/// Cap on retained log entries per job. Older entries get dropped so a
/// long-running job can't unbounded-grow the TUI's memory footprint.
pub const JOB_LOG_LIMIT: usize = 200;

/// A single per-job log entry mirrored from `Event::JobLog`. `stage` is
/// the kebab-case Runner stage name (`prepare`, `load`, `run`, ...) or
/// `None` for daemon-level logs. `ts` is always set: the backend stamps
/// every event at publish time and the wire round-trips that value.
/// Entries without a parseable ts are dropped at the wire boundary rather
/// than smuggled in with a fallback. `i18n_key`/`i18n_args` carry the
/// localizable key + named substitutions when present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobLogEntry {
    pub level: String,
    pub message: String,
    pub stage: Option<String>,
    pub ts: DateTime<Utc>,
    pub i18n_key: Option<String>,
    pub i18n_args: Vec<(String, String)>,
}

/// Live state of the TUI -> daemon connection. Reflected in the status bar
/// and used by the reconnect loop to decide whether to surface "trying to
/// reconnect" UX.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ConnectionState {
    /// Initial state: no successful connect attempt yet.
    Connecting,
    /// At least one WS handshake completed, HTTP polls succeeding.
    Connected,
    /// WS closed or HTTP poll failed. Reconnect task is retrying.
    #[default]
    Disconnected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum View {
    Jobs,
    JobDetail { id: String },
    Campaigns,
    Duts,
    About,
}

/// Reachability of a DUT's transport as reported by the daemon's `/duts`
/// probe. `Unknown` is the default when the daemon can't determine the
/// state (mock transport, optional feature disabled).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ConnectionStatus {
    Connected,
    Disconnected,
    #[default]
    Unknown,
}

/// Minimal projection of `heimdall_daemon::DutRecord` for the TUI. The daemon
/// struct itself isn't `Deserialize`, and the TUI only needs a flat view of
/// the registered DUTs + which are currently leased + live reachability.
#[derive(Debug, Clone, Deserialize)]
pub struct DutRow {
    pub id: String,
    pub kind: String,
    #[serde(default)]
    pub chip_serial: Option<String>,
    /// Free-text summary of the JTAG transport driver (e.g. "mock", "ftdi",
    /// "openocd"). Filled in from the embedded `jtag.driver` field.
    #[serde(default)]
    pub jtag_driver: Option<String>,
    /// Job id holding the lease on this DUT, if any.
    #[serde(default)]
    pub leased_by: Option<String>,
    /// Live reachability of the configured transport. `Unknown` if the
    /// daemon couldn't probe (mock transport, ftdi feature off, etc).
    #[serde(default)]
    pub connection_status: ConnectionStatus,
}

#[derive(Debug)]
pub struct App {
    pub view: View,
    pub jobs: Vec<Job>,
    pub campaigns: Vec<Campaign>,
    pub duts: Vec<DutRow>,
    pub focused_index: usize,
    pub status: String,
    pub connection: ConnectionState,
    pub should_quit: bool,
    /// Per-job log buffer. Keyed by the stringified `JobId` so the same key
    /// works whether it came in over the WS as a UUID or as the daemon's
    /// `Job::id` field. Each buffer is capped at [`JOB_LOG_LIMIT`].
    pub job_logs: HashMap<String, VecDeque<JobLogEntry>>,
    /// Jobs whose log history has been backfilled from `/jobs/:id/logs`.
    /// Once a job is in this set, the refresh loop stops re-fetching its
    /// history so live deltas from the WS aren't clobbered.
    pub backfilled_jobs: HashSet<String>,
    /// Latest register snapshots per DUT, keyed by `(dut_id, source)`.
    /// Mirrors what `/duts.snapshots` returns and gets updated live from
    /// `Event::DutStateSnapshot` frames.
    pub dut_snapshots: HashMap<(String, DutSnapshotSource), DutSnapshot>,
    /// Cached `/about` response. `None` until the About view is
    /// opened for the first time; the refresh loop fetches it on
    /// demand and stores the result here so subsequent renders are
    /// pure draws.
    pub about: Option<AboutInfoTui>,
}

/// Local mirror of `heimdall_daemon::AboutInfo` and friends. We
/// duplicate the struct rather than `serde` it off the wire crate
/// because heimdall-daemon's `AboutInfo` isn't `Deserialize` for
/// us by default (and pulling it in as a TUI dep would mean linking
/// the whole daemon). Matches the JSON shape verbatim.
#[derive(Debug, Clone, Deserialize)]
pub struct AboutInfoTui {
    pub version: String,
    pub build_profile: String,
    pub features: AboutFeaturesTui,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AboutFeaturesTui {
    #[serde(default)]
    pub sqlite: bool,
    #[serde(default)]
    pub aegis: bool,
    #[serde(default)]
    pub river: bool,
    #[serde(default)]
    pub fuzzer: bool,
    #[serde(default)]
    pub cranelift: bool,
}

impl AboutFeaturesTui {
    /// Return the enabled feature names in stable alphabetical
    /// order for rendering.
    pub fn enabled(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.aegis {
            out.push("aegis");
        }
        if self.cranelift {
            out.push("cranelift");
        }
        if self.fuzzer {
            out.push("fuzzer");
        }
        if self.river {
            out.push("river");
        }
        if self.sqlite {
            out.push("sqlite");
        }
        out
    }
}

/// Which side captured a `DutSnapshot`. Mirrors `SnapshotSourceTag` on the
/// wire so the TUI can render `dut` vs `golden` distinctively.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DutSnapshotSource {
    Dut,
    Golden,
}

impl DutSnapshotSource {
    pub fn as_str(self) -> &'static str {
        match self {
            DutSnapshotSource::Dut => "dut",
            DutSnapshotSource::Golden => "golden",
        }
    }
}

/// Returned by `<DutSnapshotSource as FromStr>::from_str` when the
/// wire-side source tag isn't one this TUI knows about. Carries the
/// offending input so callers can include it in diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown dut snapshot source `{0}`")]
pub struct UnknownDutSnapshotSource(pub String);

impl std::str::FromStr for DutSnapshotSource {
    type Err = UnknownDutSnapshotSource;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "dut" => Ok(DutSnapshotSource::Dut),
            "golden" => Ok(DutSnapshotSource::Golden),
            other => Err(UnknownDutSnapshotSource(other.to_string())),
        }
    }
}

/// Most recent state for a `(dut, source)` pair, mirrored from the server.
#[derive(Debug, Clone)]
pub struct DutSnapshot {
    pub source: DutSnapshotSource,
    pub ts: DateTime<Utc>,
    pub job: Option<String>,
    pub fields: Vec<(String, String)>,
}

impl App {
    pub fn new() -> Self {
        Self {
            view: View::Jobs,
            jobs: Vec::new(),
            campaigns: Vec::new(),
            duts: Vec::new(),
            focused_index: 0,
            status: heimdall_i18n::t("common.status.connecting"),
            connection: ConnectionState::Connecting,
            should_quit: false,
            job_logs: HashMap::new(),
            backfilled_jobs: HashSet::new(),
            dut_snapshots: HashMap::new(),
            about: None,
        }
    }

    /// Apply a `Event::DutStateSnapshot`, overwriting the previous entry
    /// for the same `(dut, source)` pair.
    pub fn apply_dut_snapshot(&mut self, dut_id: impl Into<String>, snap: DutSnapshot) {
        let key = (dut_id.into(), snap.source);
        self.dut_snapshots.insert(key, snap);
    }

    /// Return every cached snapshot for a single DUT, ordered Dut-then-Golden.
    pub fn snapshots_for_dut(&self, dut_id: &str) -> Vec<&DutSnapshot> {
        let mut v: Vec<&DutSnapshot> = self
            .dut_snapshots
            .iter()
            .filter_map(|((id, _), snap)| (id == dut_id).then_some(snap))
            .collect();
        v.sort_by_key(|s| match s.source {
            DutSnapshotSource::Dut => 0,
            DutSnapshotSource::Golden => 1,
        });
        v
    }

    /// Whether `/jobs/:id/logs` has already been pulled for this job. Used
    /// by the event loop to avoid re-fetching history on every tick.
    pub fn is_backfilled(&self, job_id: &str) -> bool {
        self.backfilled_jobs.contains(job_id)
    }

    /// Append a log entry to the per-job buffer. Drops the oldest entry when
    /// the buffer exceeds [`JOB_LOG_LIMIT`].
    pub fn push_job_log(&mut self, job_id: impl Into<String>, entry: JobLogEntry) {
        let buf = self.job_logs.entry(job_id.into()).or_default();
        buf.push_back(entry);
        while buf.len() > JOB_LOG_LIMIT {
            buf.pop_front();
        }
    }

    /// Replace the per-job buffer wholesale and mark the job as
    /// backfilled. Used by the event loop after fetching `/jobs/:id/logs`
    /// to seed a freshly-opened detail panel with persisted history.
    /// Caller-side ordering: oldest first.
    pub fn replace_job_logs(&mut self, job_id: impl Into<String>, entries: Vec<JobLogEntry>) {
        let key = job_id.into();
        let mut buf: VecDeque<JobLogEntry> = entries.into_iter().collect();
        while buf.len() > JOB_LOG_LIMIT {
            buf.pop_front();
        }
        self.job_logs.insert(key.clone(), buf);
        self.backfilled_jobs.insert(key);
    }

    /// Borrow the log buffer for a job by id string. Returns `None` if no
    /// events have arrived for that job yet.
    pub fn logs_for(&self, job_id: &str) -> Option<&VecDeque<JobLogEntry>> {
        self.job_logs.get(job_id)
    }

    /// Mark the connection as lost. Called from the event loop when the WS
    /// drops or an HTTP poll fails.
    pub fn mark_disconnected(&mut self, reason: impl Into<String>) {
        self.connection = ConnectionState::Disconnected;
        let reason = reason.into();
        self.status = if reason.is_empty() {
            heimdall_i18n::t("tui.disconnected_short")
        } else {
            heimdall_i18n::t!("tui.disconnected_reason", reason = reason)
        };
    }

    /// Mark the connection as healthy. Called when the WS reconnects.
    pub fn mark_connected(&mut self, daemon_url: &str) {
        self.connection = ConnectionState::Connected;
        self.status = heimdall_i18n::t!("tui.connected_to", url = daemon_url);
    }

    pub fn focused_job(&self) -> Option<&Job> {
        self.jobs.get(self.focused_index)
    }

    pub fn focused_campaign(&self) -> Option<&Campaign> {
        self.campaigns.get(self.focused_index)
    }

    pub fn focused_dut(&self) -> Option<&DutRow> {
        self.duts.get(self.focused_index)
    }

    pub fn navigate_down(&mut self) {
        let cap = self.focus_cap();
        if cap == 0 {
            return;
        }
        if self.focused_index + 1 < cap {
            self.focused_index += 1;
        }
    }

    pub fn navigate_up(&mut self) {
        if self.focused_index > 0 {
            self.focused_index -= 1;
        }
    }

    pub fn switch_view(&mut self, view: View) {
        self.view = view;
        self.focused_index = 0;
    }

    fn focus_cap(&self) -> usize {
        match self.view {
            View::Jobs => self.jobs.len(),
            View::Campaigns => self.campaigns.len(),
            View::Duts => self.duts.len(),
            View::JobDetail { .. } => 0,
            View::About => 0,
        }
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_clamps_within_jobs() {
        let mut app = App::new();
        // Synthesize three jobs via JSON for brevity.
        app.jobs = serde_json::from_str(r#"
            [
                {"id":"00000000-0000-0000-0000-000000000001","dut":"d1","dut_kind":"river-rc1-nano","kind":{"kind":"mock-hello"},"campaign":null,"state":{"state":"queued"},"created_at":"2026-05-25T00:00:00Z","updated_at":"2026-05-25T00:00:00Z"},
                {"id":"00000000-0000-0000-0000-000000000002","dut":"d2","dut_kind":"river-rc1-nano","kind":{"kind":"mock-hello"},"campaign":null,"state":{"state":"queued"},"created_at":"2026-05-25T00:00:00Z","updated_at":"2026-05-25T00:00:00Z"},
                {"id":"00000000-0000-0000-0000-000000000003","dut":"d3","dut_kind":"river-rc1-nano","kind":{"kind":"mock-hello"},"campaign":null,"state":{"state":"queued"},"created_at":"2026-05-25T00:00:00Z","updated_at":"2026-05-25T00:00:00Z"}
            ]
        "#).unwrap();
        for _ in 0..5 {
            app.navigate_down();
        }
        assert_eq!(app.focused_index, 2);
        for _ in 0..5 {
            app.navigate_up();
        }
        assert_eq!(app.focused_index, 0);
    }

    #[test]
    fn switch_view_resets_focus() {
        let mut app = App::new();
        app.focused_index = 5;
        app.switch_view(View::Campaigns);
        assert_eq!(app.focused_index, 0);
    }

    #[test]
    fn duts_view_navigation_clamps_within_list() {
        let mut app = App::new();
        app.switch_view(View::Duts);
        app.duts = vec![
            DutRow {
                id: "d1".into(),
                kind: "river-rc1-nano".into(),
                chip_serial: None,
                jtag_driver: Some("mock".into()),
                leased_by: None,
                connection_status: ConnectionStatus::Unknown,
            },
            DutRow {
                id: "d2".into(),
                kind: "aegis-luna-1".into(),
                chip_serial: Some("SN-2".into()),
                jtag_driver: Some("ftdi".into()),
                leased_by: Some("00000000-0000-0000-0000-000000000001".into()),
                connection_status: ConnectionStatus::Connected,
            },
        ];
        for _ in 0..5 {
            app.navigate_down();
        }
        assert_eq!(app.focused_index, 1);
        assert_eq!(app.focused_dut().unwrap().id, "d2");
    }

    #[test]
    fn push_job_log_appends_until_cap_then_drops_oldest() {
        let mut app = App::new();
        for i in 0..(JOB_LOG_LIMIT + 5) {
            app.push_job_log(
                "job-x",
                JobLogEntry {
                    level: "info".into(),
                    message: format!("m{i}"),
                    stage: Some("prepare".into()),
                    ts: Utc::now(),
                    i18n_key: None,
                    i18n_args: Vec::new(),
                },
            );
        }
        let buf = app.logs_for("job-x").expect("buffer present");
        assert_eq!(buf.len(), JOB_LOG_LIMIT);
        assert_eq!(buf.front().unwrap().message, format!("m{}", 5));
        assert_eq!(
            buf.back().unwrap().message,
            format!("m{}", JOB_LOG_LIMIT + 4)
        );
    }

    #[test]
    fn logs_for_unknown_job_returns_none() {
        let app = App::new();
        assert!(app.logs_for("missing").is_none());
    }

    #[test]
    fn replace_job_logs_marks_backfilled_and_caps_buffer() {
        let mut app = App::new();
        assert!(!app.is_backfilled("job-y"));
        let many: Vec<_> = (0..(JOB_LOG_LIMIT + 5))
            .map(|i| JobLogEntry {
                level: "info".into(),
                message: format!("hist {i}"),
                stage: None,
                ts: Utc::now(),
                i18n_key: None,
                i18n_args: Vec::new(),
            })
            .collect();
        app.replace_job_logs("job-y", many);
        assert!(app.is_backfilled("job-y"));
        let buf = app.logs_for("job-y").expect("buffer");
        assert_eq!(buf.len(), JOB_LOG_LIMIT);
        // Oldest entries get trimmed, newest survive.
        assert_eq!(
            buf.back().unwrap().message,
            format!("hist {}", JOB_LOG_LIMIT + 4)
        );
    }

    #[test]
    fn focused_dut_is_none_when_list_empty() {
        let mut app = App::new();
        app.switch_view(View::Duts);
        assert!(app.focused_dut().is_none());
    }
}
