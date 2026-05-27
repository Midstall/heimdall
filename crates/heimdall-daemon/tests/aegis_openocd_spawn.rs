//! Integration test for the OpenocdSpawned transport spec dispatch via
//! AegisRealFactory. The test uses `sleep` as the "openocd binary" so the
//! Tcl port never becomes reachable; we expect the worker to time out the
//! transport open and report Failed.

#![cfg(all(feature = "sqlite", feature = "aegis"))]

use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use heimdall_config::{
    ConfigFile, DutCfg, GoldenBackendCfg, GoldenCfg, HostCfg, JtagDriver, JtagTransportCfg,
    ToolsCfg, TransportSection,
};
use heimdall_core::DutKind;
use heimdall_daemon::{BlobStore, JobStore, LocalFsBlobStore, SqliteJobStore, runtime};
use serde_json::json;
use tempfile::TempDir;

#[tokio::test]
async fn openocd_spawn_failure_surfaces_as_job_failed() {
    let tmp = TempDir::new().expect("tmp");
    let store = SqliteJobStore::open_in_memory().await.expect("store");
    let blobs = LocalFsBlobStore::open(tmp.path().to_path_buf())
        .await
        .expect("blobs");

    // Sleep for 60s on a config file we don't have; sleep ignores the arg
    // but exits 0 on its own argument parsing error. Whatever its behavior,
    // it won't bind port 55323 within the timeout. The transport's open()
    // should return a Timeout error which the worker maps to JobState::Failed.
    let cfg = ConfigFile {
        host: HostCfg {
            name: "rig-01".into(),
            bind: "127.0.0.1:0".parse().unwrap(),
        },
        duts: vec![DutCfg {
            id: "luna1-1".into(),
            kind: DutKind::AegisLuna1,
            chip_serial: None,
            transports: vec!["jtag.spawn".into()],
            expect_idcode: None,
            bringup: None,
            netlist: None,
            spice_watches: vec![],
        }],
        transport: TransportSection {
            jtag: vec![JtagTransportCfg {
                id: "jtag.spawn".into(),
                driver: JtagDriver::OpenocdSpawn,
                serial: None,
                openocd_endpoint: Some("127.0.0.1:55323".parse().unwrap()),
                openocd_config: Some("/dev/null".into()),
                openocd_binary: Some("sleep".into()),
                openocd_extra_args: vec!["30".into()],
                freq_hz: 1_000_000,
                ftdi_vid: None,
                ftdi_pid: None,
                ftdi_interface: None,
            }],
            ..Default::default()
        },
        golden: GoldenCfg {
            aegis: Some(GoldenBackendCfg::Mock),
            river: None,
        },
        tools: ToolsCfg::default(),
        pad_maps: vec![],
    };

    let bind: std::net::SocketAddr = "127.0.0.1:0".parse().unwrap();
    let handles = runtime::start_with_config(
        bind,
        Arc::new(store) as Arc<dyn JobStore>,
        Arc::new(blobs) as Arc<dyn BlobStore>,
        &cfg,
    )
    .await
    .expect("daemon start");

    let client = reqwest::Client::new();
    let bitstream = vec![0u8; 30];
    let bitstream_b64 = base64::engine::general_purpose::STANDARD.encode(&bitstream);
    let descriptor_json = r#"{
        "device": "test_fpga",
        "fabric": {
            "width": 2, "height": 2, "tracks": 1, "tile_config_width": 46,
            "bram": {"column_interval":0,"columns":[],"data_width":null,"addr_width":null,"depth":null,"tile_config_width":8},
            "dsp":  {"column_interval":0,"columns":[],"a_width":null,"b_width":null,"result_width":null,"tile_config_width":16},
            "carry_chain": {"direction":"south_to_north","per_column":true}
        },
        "io": {"total_pads": 8, "tile_config_width": 8, "pads": []},
        "serdes": {"count":0,"tile_config_width":32,"edge_assignment":[]},
        "clock":  {"tile_count":1,"tile_config_width":49,"outputs_per_tile":4,"total_outputs":4},
        "config": {"total_bits":233,"chain_order":[]},
        "tiles": []
    }"#;
    let body = json!({
        "dut": "luna1-1",
        "kind": {
            "kind": "load-aegis-bitstream",
            "descriptor_json": descriptor_json,
            "bitstream_b64": bitstream_b64,
        }
    });
    let url = format!("http://{}/jobs", handles.local_addr);
    let resp = client.post(&url).json(&body).send().await.expect("post");
    assert_eq!(resp.status(), 201);
    let job: serde_json::Value = resp.json().await.expect("json");
    let job_id = job["id"].as_str().unwrap().to_string();

    // The spawned transport has a 10s startup timeout; we wait up to 15s.
    let url = format!("http://{}/jobs/{job_id}", handles.local_addr);
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut last_state = String::new();
    let mut reached = false;
    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let resp = client.get(&url).send().await.expect("get");
        if resp.status() == 200 {
            let body: serde_json::Value = resp.json().await.expect("json");
            last_state = body["state"]["state"].as_str().unwrap_or("").to_string();
            if matches!(last_state.as_str(), "done" | "failed") {
                reached = true;
                break;
            }
        }
    }
    assert!(reached, "job never reached terminal; last `{last_state}`");
    assert_eq!(
        last_state, "failed",
        "expected Failed because transport open times out; got `{last_state}`"
    );

    handles.server_task.abort();
    handles.worker_task.abort();
}
