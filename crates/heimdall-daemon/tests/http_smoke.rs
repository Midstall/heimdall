//! End-to-end smoke test of the daemon over real HTTP. Spawns the daemon
//! against an in-memory SqliteJobStore + tempdir LocalFsBlobStore, ephemeral
//! TCP port, then drives the API via reqwest.

#![cfg(feature = "sqlite")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use heimdall_core::DutId;
use heimdall_daemon::{
    BlobStore, JobKind, JobStore, LocalFsBlobStore, NewJob, SqliteJobStore, runtime,
};
use tempfile::TempDir;

async fn start_daemon() -> (heimdall_daemon::DaemonHandles, TempDir) {
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

fn base_url(addr: std::net::SocketAddr) -> String {
    format!("http://{addr}")
}

#[tokio::test]
async fn health_returns_ok() {
    let (handles, _tmp) = start_daemon().await;
    let url = format!("{}/health", base_url(handles.local_addr));
    let resp = reqwest::get(&url).await.expect("get");
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.expect("json");
    assert_eq!(body["status"], "ok");
    handles.server_task.abort();
    handles.worker_task.abort();
}

#[tokio::test]
async fn create_get_job_reaches_terminal_state() {
    let (handles, _tmp) = start_daemon().await;
    let client = reqwest::Client::new();

    // POST a MockHello job.
    let new_job = NewJob {
        dut: DutId::new("mock-dut"),
        kind: JobKind::MockHello,
        campaign: None,
    };
    let resp = client
        .post(format!("{}/jobs", base_url(handles.local_addr)))
        .json(&new_job)
        .send()
        .await
        .expect("post");
    assert_eq!(resp.status(), 201, "expected 201 Created");
    let job: serde_json::Value = resp.json().await.expect("created json");
    let job_id = job["id"].as_str().expect("id").to_string();

    // Poll until terminal state.
    let url = format!("{}/jobs/{job_id}", base_url(handles.local_addr));
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut last_state = String::new();
    let mut reached_terminal = false;
    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let resp = client.get(&url).send().await.expect("get");
        if resp.status() == 200 {
            let body: serde_json::Value = resp.json().await.expect("json");
            last_state = body["state"]["state"].as_str().unwrap_or("").to_string();
            if matches!(last_state.as_str(), "done" | "failed" | "cancelled") {
                reached_terminal = true;
                // Done state should include the verdict. Verify it.
                if last_state == "done" {
                    let detail = &body["state"]["detail"];
                    assert_eq!(detail["kind"], "pass", "expected pass, got {body}");
                }
                break;
            }
        }
    }
    assert!(
        reached_terminal,
        "job did not reach terminal state; last was `{last_state}`"
    );
    handles.server_task.abort();
    handles.worker_task.abort();
}

#[tokio::test]
async fn list_filters_by_state() {
    let (handles, _tmp) = start_daemon().await;
    let client = reqwest::Client::new();

    // Create two jobs.
    for i in 0..2 {
        let resp = client
            .post(format!("{}/jobs", base_url(handles.local_addr)))
            .json(&NewJob {
                dut: DutId::new(format!("dut-{i}")),
                kind: JobKind::MockHello,
                campaign: None,
            })
            .send()
            .await
            .expect("post");
        assert_eq!(resp.status(), 201);
    }

    // Wait for them to finish.
    tokio::time::sleep(Duration::from_millis(300)).await;

    // List all jobs.
    let url = format!("{}/jobs", base_url(handles.local_addr));
    let resp = client.get(&url).send().await.expect("get list");
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.expect("json");
    let jobs = body["jobs"].as_array().expect("jobs array");
    assert_eq!(jobs.len(), 2);

    // List filtered to done state.
    let url = format!("{}/jobs?state=done", base_url(handles.local_addr));
    let resp = client.get(&url).send().await.expect("get filtered");
    assert_eq!(resp.status(), 200);
    let filtered: serde_json::Value = resp.json().await.expect("filtered json");
    let filtered_jobs = filtered["jobs"].as_array().expect("filtered jobs array");
    assert_eq!(filtered_jobs.len(), 2, "both jobs should be done by now");

    handles.server_task.abort();
    handles.worker_task.abort();
}

#[tokio::test]
async fn get_unknown_job_returns_404() {
    let (handles, _tmp) = start_daemon().await;
    let nonexistent = uuid::Uuid::nil();
    let url = format!("{}/jobs/{nonexistent}", base_url(handles.local_addr));
    let resp = reqwest::get(&url).await.expect("get");
    assert_eq!(resp.status(), 404);
    handles.server_task.abort();
    handles.worker_task.abort();
}
