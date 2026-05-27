//! HTTP integration test for the LoadAegisBitstream JobKind. POSTs a job,
//! waits for it to complete, expects Done(Pass).

#![cfg(all(feature = "sqlite", feature = "aegis"))]

use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
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

fn descriptor_json() -> &'static str {
    r#"{
        "device": "test_fpga",
        "fabric": {
            "width": 2,
            "height": 2,
            "tracks": 1,
            "tile_config_width": 46,
            "bram": {
                "column_interval": 0,
                "columns": [],
                "data_width": null,
                "addr_width": null,
                "depth": null,
                "tile_config_width": 8
            },
            "dsp": {
                "column_interval": 0,
                "columns": [],
                "a_width": null,
                "b_width": null,
                "result_width": null,
                "tile_config_width": 16
            },
            "carry_chain": {
                "direction": "south_to_north",
                "per_column": true
            }
        },
        "io": {
            "total_pads": 8,
            "tile_config_width": 8,
            "pads": []
        },
        "serdes": {
            "count": 0,
            "tile_config_width": 32,
            "edge_assignment": []
        },
        "clock": {
            "tile_count": 1,
            "tile_config_width": 49,
            "outputs_per_tile": 4,
            "total_outputs": 4
        },
        "config": {
            "total_bits": 233,
            "chain_order": []
        },
        "tiles": []
    }"#
}

#[tokio::test]
async fn load_aegis_bitstream_via_http_completes() {
    let (handles, _tmp) = start_daemon().await;
    let client = reqwest::Client::new();

    // Build a zero bitstream sized for total_bits = 233 (30 bytes).
    let bitstream = vec![0u8; 233u32.div_ceil(8) as usize];
    let bitstream_b64 = base64::engine::general_purpose::STANDARD.encode(&bitstream);

    let body = json!({
        "dut": "luna1-1",
        "kind": {
            "kind": "load-aegis-bitstream",
            "descriptor_json": descriptor_json(),
            "bitstream_b64": bitstream_b64,
        }
    });
    let url = format!("http://{}/jobs", handles.local_addr);
    let resp = client.post(&url).json(&body).send().await.expect("post");
    assert_eq!(resp.status(), 201, "expected 201, got {}", resp.status());
    let job: serde_json::Value = resp.json().await.expect("json");
    let job_id = job["id"].as_str().expect("id").to_string();

    // Poll for completion.
    let url = format!("http://{}/jobs/{job_id}", handles.local_addr);
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut last_state = String::new();
    let mut reached = false;
    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let resp = client.get(&url).send().await.expect("get");
        if resp.status() == 200 {
            let body: serde_json::Value = resp.json().await.expect("json");
            last_state = body["state"]["state"].as_str().unwrap_or("").to_string();
            if matches!(last_state.as_str(), "done" | "failed") {
                if last_state == "done" {
                    let detail_kind = body["state"]["detail"]["kind"].as_str().unwrap_or("");
                    assert_eq!(detail_kind, "pass", "expected pass, got: {body}");
                }
                reached = true;
                break;
            }
        }
    }
    assert!(
        reached,
        "job never reached terminal; last state `{last_state}`"
    );
    assert_eq!(last_state, "done", "expected done, got {last_state}");

    handles.server_task.abort();
    handles.worker_task.abort();
}
