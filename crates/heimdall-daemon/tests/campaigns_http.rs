//! End-to-end smoke test of campaign HTTP endpoints.

#![cfg(feature = "sqlite")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use heimdall_daemon::{BlobStore, JobStore, LocalFsBlobStore, SqliteJobStore, runtime};
use serde_json::json;
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
async fn submit_bringup_campaign_reaches_terminal_state() {
    let (handles, _tmp) = start_daemon().await;
    let client = reqwest::Client::new();

    let body = json!({
        "dut": "mock-dut",
        "template": {"kind": "bring-up"},
        "chip_serial": "TEST-0001"
    });

    let resp = client
        .post(format!("{}/campaigns", base_url(handles.local_addr)))
        .json(&body)
        .send()
        .await
        .expect("post");
    assert_eq!(resp.status(), 201, "expected 201 Created");

    let campaign: serde_json::Value = resp.json().await.expect("created json");
    let campaign_id = campaign["id"].as_str().expect("id").to_string();

    // Poll until terminal.
    let url = format!("{}/campaigns/{campaign_id}", base_url(handles.local_addr));
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut last_state = String::new();
    let mut reached_terminal = false;
    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let resp = client.get(&url).send().await.expect("get");
        if resp.status() == 200 {
            let body: serde_json::Value = resp.json().await.expect("json");
            last_state = body["state"]["state"].as_str().unwrap_or("").to_string();
            if matches!(last_state.as_str(), "pass" | "fail" | "mixed" | "cancelled") {
                reached_terminal = true;
                assert_eq!(
                    last_state, "pass",
                    "expected campaign Pass; got {last_state}"
                );
                break;
            }
        }
    }
    assert!(
        reached_terminal,
        "campaign never reached terminal state; last `{last_state}`"
    );

    // Fetch report.
    let url = format!(
        "{}/campaigns/{campaign_id}/report.json",
        base_url(handles.local_addr)
    );
    let resp = client.get(&url).send().await.expect("get report");
    assert_eq!(resp.status(), 200);
    let report: serde_json::Value = resp.json().await.expect("report json");
    assert_eq!(report["campaign"]["id"].as_str().unwrap(), campaign_id);
    let jobs = report["jobs"].as_array().expect("jobs array");
    assert_eq!(jobs.len(), 1, "BringUp should produce one job");

    handles.server_task.abort();
    handles.worker_task.abort();
}

#[tokio::test]
async fn list_campaigns_returns_array() {
    let (handles, _tmp) = start_daemon().await;
    let client = reqwest::Client::new();

    let _ = client
        .post(format!("{}/campaigns", base_url(handles.local_addr)))
        .json(&json!({
            "dut": "mock-dut",
            "template": {"kind": "bring-up"}
        }))
        .send()
        .await
        .expect("post");

    let url = format!("{}/campaigns", base_url(handles.local_addr));
    let resp = client.get(&url).send().await.expect("get list");
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.expect("json");
    let arr = body["campaigns"].as_array().expect("campaigns array");
    assert_eq!(arr.len(), 1);

    handles.server_task.abort();
    handles.worker_task.abort();
}

#[tokio::test]
async fn unknown_campaign_returns_404() {
    let (handles, _tmp) = start_daemon().await;
    let nonexistent = uuid::Uuid::nil();
    let url = format!("{}/campaigns/{nonexistent}", base_url(handles.local_addr));
    let resp = reqwest::get(&url).await.expect("get");
    assert_eq!(resp.status(), 404);
    handles.server_task.abort();
    handles.worker_task.abort();
}

#[tokio::test]
async fn unknown_campaign_report_returns_404() {
    let (handles, _tmp) = start_daemon().await;
    let nonexistent = uuid::Uuid::nil();
    let url = format!(
        "{}/campaigns/{nonexistent}/report.json",
        base_url(handles.local_addr)
    );
    let resp = reqwest::get(&url).await.expect("get");
    assert_eq!(resp.status(), 404);
    handles.server_task.abort();
    handles.worker_task.abort();
}
