//! Integration tests for the TUI reconnect loop.
//!
//! These exercise `ws_reconnect_loop` directly so we don't need a real
//! terminal. The HTTP-poll path (which independently detects disconnection
//! via failed `list_jobs` etc) is covered by unit tests in `event::tests`.

#![cfg(feature = "sqlite")]

use std::sync::Arc;
use std::time::Duration;

use heimdall_daemon::{BlobStore, JobStore, LocalFsBlobStore, SqliteJobStore, runtime};
use heimdall_tui::{AppEvent, DaemonClient, ws_reconnect_loop};
use tempfile::TempDir;
use tokio::sync::mpsc;
use tokio::time::timeout;

async fn wait_for<F>(
    events: &mut mpsc::UnboundedReceiver<AppEvent>,
    mut pred: F,
    label: &str,
    deadline: Duration,
) where
    F: FnMut(&AppEvent) -> bool,
{
    let result = timeout(deadline, async {
        while let Some(ev) = events.recv().await {
            if pred(&ev) {
                return;
            }
        }
    })
    .await;
    if result.is_err() {
        panic!("timed out waiting for {label}");
    }
}

#[tokio::test]
async fn ws_loop_emits_connection_lost_for_unreachable_url() {
    // Point at a port nothing is listening on. The first subscribe should
    // fail and the loop should emit ConnectionLost.
    let client = DaemonClient::new("http://127.0.0.1:1".to_string());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let _task = tokio::spawn({
        let client = client.clone();
        async move { ws_reconnect_loop(client, tx).await }
    });

    wait_for(
        &mut rx,
        |ev| matches!(ev, AppEvent::ConnectionLost { .. }),
        "ConnectionLost",
        Duration::from_secs(5),
    )
    .await;
}

#[tokio::test]
async fn ws_loop_retries_with_backoff_until_daemon_appears() {
    // Phase 1: launch the loop pointing at a (currently empty) port.
    let bind: std::net::SocketAddr = "127.0.0.1:0".parse().unwrap();
    // First grab an unused port by binding a probe listener and then
    // dropping it. Race window is small enough for a test.
    let probe = tokio::net::TcpListener::bind(bind).await.unwrap();
    let target = probe.local_addr().unwrap();
    drop(probe);

    let client = DaemonClient::new(format!("http://{target}"));
    let (tx, mut rx) = mpsc::unbounded_channel();
    let _task = tokio::spawn({
        let client = client.clone();
        async move { ws_reconnect_loop(client, tx).await }
    });

    // We expect at least one ConnectionLost while no daemon is listening.
    wait_for(
        &mut rx,
        |ev| matches!(ev, AppEvent::ConnectionLost { .. }),
        "ConnectionLost (no daemon)",
        Duration::from_secs(5),
    )
    .await;

    // Phase 2: stand up a real daemon on the same port. The loop should
    // pick it up on the next backoff tick and emit ConnectionRestored.
    let tmp = TempDir::new().unwrap();
    let store = SqliteJobStore::open_in_memory().await.unwrap();
    let blobs = LocalFsBlobStore::open(tmp.path().to_path_buf())
        .await
        .unwrap();
    let handles = runtime::start(
        target,
        Arc::new(store) as Arc<dyn JobStore>,
        Arc::new(blobs) as Arc<dyn BlobStore>,
    )
    .await
    .expect("daemon start");

    wait_for(
        &mut rx,
        |ev| matches!(ev, AppEvent::ConnectionRestored),
        "ConnectionRestored",
        // Backoff caps at 30s but should fire within a few retries.
        Duration::from_secs(60),
    )
    .await;

    handles.server_task.abort();
    handles.worker_task.abort();
}
