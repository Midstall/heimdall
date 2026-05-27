//! Integration test for the Prometheus /metrics endpoint.

#![cfg(feature = "sqlite")]

use std::sync::Arc;

use heimdall_daemon::{BlobStore, JobStore, LocalFsBlobStore, SqliteJobStore, runtime};
use tempfile::TempDir;

async fn start() -> (heimdall_daemon::DaemonHandles, TempDir) {
    let tmp = TempDir::new().expect("tmp");
    let store = SqliteJobStore::open_in_memory().await.expect("store");
    let blobs = LocalFsBlobStore::open(tmp.path().to_path_buf())
        .await
        .expect("blobs");
    let bind: std::net::SocketAddr = "127.0.0.1:0".parse().unwrap();
    let handles = runtime::start(
        bind,
        Arc::new(store) as Arc<dyn JobStore>,
        Arc::new(blobs) as Arc<dyn BlobStore>,
    )
    .await
    .expect("daemon start");
    (handles, tmp)
}

#[tokio::test]
async fn metrics_endpoint_returns_prometheus_text() {
    let (handles, _tmp) = start().await;
    let url = format!("http://{}/metrics", handles.local_addr);
    let resp = reqwest::get(&url).await.expect("get");
    assert_eq!(resp.status(), 200);
    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(
        ct.starts_with("text/plain"),
        "expected text/plain content-type, got `{ct}`"
    );
    let body = resp.text().await.expect("text");

    // Every gauge/counter we promised must be in the body.
    for needle in [
        "heimdall_build_info",
        "heimdall_uptime_seconds",
        "heimdall_jobs{state=\"queued\"}",
        "heimdall_jobs{state=\"running\"}",
        "heimdall_jobs{state=\"done\"}",
        "heimdall_jobs{state=\"failed\"}",
        "heimdall_jobs{state=\"cancelled\"}",
        "heimdall_verdicts{result=\"pass\"}",
        "heimdall_verdicts{result=\"fail\"}",
        "heimdall_duts_configured",
        "heimdall_leases_active",
    ] {
        assert!(
            body.contains(needle),
            "missing `{needle}` in metrics body:\n{body}"
        );
    }

    // # HELP/# TYPE conformance.
    assert!(body.contains("# HELP heimdall_uptime_seconds"));
    assert!(body.contains("# TYPE heimdall_jobs gauge"));

    handles.server_task.abort();
    handles.worker_task.abort();
}

#[tokio::test]
async fn metrics_reflects_zero_state_on_fresh_daemon() {
    let (handles, _tmp) = start().await;
    let url = format!("http://{}/metrics", handles.local_addr);
    let resp = reqwest::get(&url).await.expect("get");
    let body = resp.text().await.expect("text");

    // Fresh daemon: no jobs, no leases, no DUTs (because runtime::start uses
    // an empty DutRegistry).
    assert!(body.contains("heimdall_jobs{state=\"queued\"} 0"));
    assert!(body.contains("heimdall_jobs{state=\"done\"} 0"));
    assert!(body.contains("heimdall_duts_configured 0"));
    assert!(body.contains("heimdall_leases_active 0"));

    handles.server_task.abort();
    handles.worker_task.abort();
}
