//! End-to-end test: a MockHello job emits per-stage `job-log` events over
//! the `/events` WebSocket. Validates that the StageObserver -> JobLogger ->
//! EventBus -> WebSocket path stays wired end-to-end.

#![cfg(feature = "sqlite")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::{SinkExt, StreamExt};
use heimdall_core::{DutId, DutKind};
use heimdall_daemon::{
    BlobStore, DutRecord, DutRegistry, JobKind, JobStore, LocalFsBlobStore, NewJob, SqliteJobStore,
    runtime,
};
use tempfile::TempDir;
use tokio_tungstenite::{connect_async, tungstenite};

async fn start_daemon() -> (heimdall_daemon::DaemonHandles, TempDir) {
    let tmp = TempDir::new().expect("tmp");
    let store = SqliteJobStore::open_in_memory().await.expect("store");
    let blobs = LocalFsBlobStore::open(tmp.path().to_path_buf())
        .await
        .expect("blobs");
    let mut registry = DutRegistry::new();
    registry.insert(DutRecord::mock("log-dut", DutKind::RiverRc1Nano));
    let bind: std::net::SocketAddr = "127.0.0.1:0".parse().unwrap();
    let handles = runtime::start_with_dut_registry(
        bind,
        Arc::new(store) as Arc<dyn JobStore>,
        Arc::new(blobs) as Arc<dyn BlobStore>,
        Arc::new(registry),
    )
    .await
    .expect("daemon start");
    (handles, tmp)
}

#[tokio::test]
async fn mock_hello_emits_stage_tagged_job_logs() {
    let (handles, _tmp) = start_daemon().await;

    // Subscribe to /events FIRST so we don't miss early stage events.
    let ws_url = format!("ws://{}/events", handles.local_addr);
    let (mut ws, _) = connect_async(&ws_url).await.expect("ws connect");

    // Post a MockHello job.
    let client = reqwest::Client::new();
    let url = format!("http://{}/jobs", handles.local_addr);
    let resp = client
        .post(&url)
        .json(&NewJob {
            dut: DutId::new("log-dut"),
            kind: JobKind::MockHello,
            campaign: None,
        })
        .send()
        .await
        .expect("post");
    assert_eq!(resp.status(), 201);

    // Collect events until we see at least one job-log per stage we care about.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut seen_prepare = false;
    let mut seen_diff = false;
    let mut seen_dispatch = false;
    while Instant::now() < deadline && !(seen_prepare && seen_diff && seen_dispatch) {
        let recv = tokio::time::timeout(Duration::from_millis(250), ws.next()).await;
        let frame = match recv {
            Ok(Some(Ok(f))) => f,
            _ => continue,
        };
        let text = match frame {
            tungstenite::Message::Text(t) => t,
            _ => continue,
        };
        let v: serde_json::Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => continue,
        };
        // Every broadcast frame is a StampedEvent: `ts` flat-flattened
        // alongside the event variant fields.
        assert!(v["ts"].is_string(), "missing ts on frame: {v}");
        if v["kind"] != "job-log" {
            continue;
        }
        let stage = v["stage"].as_str();
        let message = v["message"].as_str().unwrap_or("");
        match stage {
            Some("prepare") => seen_prepare = true,
            Some("diff") => seen_diff = true,
            None if message.contains("dispatching") => seen_dispatch = true,
            _ => {}
        }
    }

    assert!(seen_dispatch, "expected daemon-level dispatch log");
    assert!(seen_prepare, "expected stage=prepare log");
    assert!(seen_diff, "expected stage=diff log");

    let _ = ws.send(tungstenite::Message::Close(None)).await;

    // After the run completes, /jobs/:id/logs should return the persisted
    // history so a late-joining client can backfill.
    let job_id = {
        let jobs: serde_json::Value = client
            .get(format!("http://{}/jobs", handles.local_addr))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        jobs["jobs"][0]["id"].as_str().unwrap().to_string()
    };
    let history: serde_json::Value = client
        .get(format!("http://{}/jobs/{job_id}/logs", handles.local_addr))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let logs = history["logs"].as_array().expect("logs array");
    assert!(
        !logs.is_empty(),
        "expected persisted job logs, got empty array"
    );
    let kinds: Vec<&str> = logs
        .iter()
        .map(|l| l["kind"].as_str().unwrap_or(""))
        .collect();
    assert!(
        kinds.iter().all(|k| *k == "job-log"),
        "all entries should be job-log events; got {kinds:?}"
    );
    let stages: Vec<&str> = logs.iter().filter_map(|l| l["stage"].as_str()).collect();
    assert!(stages.contains(&"prepare"), "history missing prepare stage");
    assert!(stages.contains(&"diff"), "history missing diff stage");
    // Every entry carries a parseable UTC timestamp.
    for l in logs {
        let ts = l["ts"].as_str().expect("ts present on each entry");
        chrono::DateTime::parse_from_rfc3339(ts).expect("ts parses as RFC3339");
    }

    // since=<id> returns only events strictly newer than that id.
    let last_id = logs.last().unwrap()["id"].as_u64().unwrap();
    let empty_delta: serde_json::Value = client
        .get(format!(
            "http://{}/jobs/{job_id}/logs?since={last_id}",
            handles.local_addr
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        empty_delta["logs"].as_array().unwrap().len(),
        0,
        "since=last_id should return zero rows"
    );

    handles.server_task.abort();
    handles.worker_task.abort();
}
