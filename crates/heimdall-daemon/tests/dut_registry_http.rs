//! Integration test verifying that GET /duts returns the configured DUT
//! registry when the daemon is started via start_with_config.

#![cfg(all(feature = "sqlite", feature = "aegis"))]

use std::sync::Arc;

use heimdall_config::{
    ConfigFile, DutCfg, GoldenBackendCfg, GoldenCfg, HostCfg, JtagDriver, JtagTransportCfg,
    ToolsCfg, TransportSection,
};
use heimdall_core::DutKind;
use heimdall_daemon::{BlobStore, JobStore, LocalFsBlobStore, SqliteJobStore, runtime};
use tempfile::TempDir;

async fn start_with_two_duts() -> (heimdall_daemon::DaemonHandles, TempDir) {
    let tmp = TempDir::new().expect("tmp");
    let store = SqliteJobStore::open_in_memory().await.expect("store");
    let blobs = LocalFsBlobStore::open(tmp.path().to_path_buf())
        .await
        .expect("blobs");

    let cfg = ConfigFile {
        host: HostCfg {
            name: "rig-01".into(),
            bind: "127.0.0.1:0".parse().unwrap(),
        },
        duts: vec![
            DutCfg {
                id: "luna1-1".into(),
                kind: DutKind::AegisLuna1,
                chip_serial: Some("L1-0001".into()),
                transports: vec!["jtag.mock1".into()],
                expect_idcode: None,
                bringup: None,
                netlist: None,
                spice_watches: vec![],
            },
            DutCfg {
                id: "luna1-2".into(),
                kind: DutKind::AegisLuna1,
                chip_serial: None,
                transports: vec!["jtag.ocd1".into()],
                expect_idcode: None,
                bringup: None,
                netlist: None,
                spice_watches: vec![],
            },
        ],
        transport: TransportSection {
            jtag: vec![
                JtagTransportCfg {
                    id: "jtag.mock1".into(),
                    driver: JtagDriver::Mock,
                    serial: None,
                    openocd_endpoint: None,
                    openocd_config: None,
                    openocd_binary: None,
                    openocd_extra_args: vec![],
                    freq_hz: 1_000_000,
                    ftdi_vid: None,
                    ftdi_pid: None,
                    ftdi_interface: None,
                },
                JtagTransportCfg {
                    id: "jtag.ocd1".into(),
                    driver: JtagDriver::Openocd,
                    serial: None,
                    openocd_endpoint: Some("127.0.0.1:6666".parse().unwrap()),
                    openocd_config: None,
                    openocd_binary: None,
                    openocd_extra_args: vec![],
                    freq_hz: 1_000_000,
                    ftdi_vid: None,
                    ftdi_pid: None,
                    ftdi_interface: None,
                },
            ],
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
    (handles, tmp)
}

#[tokio::test]
async fn duts_endpoint_returns_configured_duts() {
    let (handles, _tmp) = start_with_two_duts().await;
    let url = format!("http://{}/duts", handles.local_addr);
    let resp = reqwest::get(&url).await.expect("get");
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.expect("json");
    let duts = body["duts"].as_array().expect("duts array");
    assert_eq!(duts.len(), 2);

    let by_id: std::collections::BTreeMap<&str, &serde_json::Value> = duts
        .iter()
        .map(|d| (d["id"].as_str().unwrap(), d))
        .collect();
    let mock = by_id.get("luna1-1").unwrap();
    assert_eq!(mock["jtag"]["driver"].as_str().unwrap(), "mock");
    assert_eq!(mock["chip_serial"].as_str().unwrap(), "L1-0001");
    let ocd = by_id.get("luna1-2").unwrap();
    assert_eq!(ocd["jtag"]["driver"].as_str().unwrap(), "openocd");
    assert_eq!(ocd["jtag"]["endpoint"].as_str().unwrap(), "127.0.0.1:6666");

    handles.server_task.abort();
    handles.worker_task.abort();
}

#[tokio::test]
async fn load_aegis_via_configured_mock_completes() {
    use base64::Engine;
    let (handles, _tmp) = start_with_two_duts().await;
    let client = reqwest::Client::new();

    let bitstream = vec![0u8; (233u32).div_ceil(8) as usize];
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

    let body = serde_json::json!({
        "dut": "luna1-1",
        "kind": {
            "kind": "load-aegis-bitstream",
            "descriptor_json": descriptor_json,
            "bitstream_b64": bitstream_b64,
        }
    });
    let url = format!("http://{}/jobs", handles.local_addr);
    let resp = client.post(&url).json(&body).send().await.expect("post");
    assert_eq!(resp.status(), 201, "expected 201; got {}", resp.status());
    let job: serde_json::Value = resp.json().await.expect("json");
    let job_id = job["id"].as_str().unwrap().to_string();

    // Poll for completion.
    let url = format!("http://{}/jobs/{job_id}", handles.local_addr);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut last_state = String::new();
    let mut reached = false;
    while std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let resp = client.get(&url).send().await.expect("get");
        if resp.status() == 200 {
            let body: serde_json::Value = resp.json().await.expect("json");
            last_state = body["state"]["state"].as_str().unwrap_or("").to_string();
            if matches!(last_state.as_str(), "done" | "failed") {
                if last_state == "done" {
                    let detail_kind = body["state"]["detail"]["kind"].as_str().unwrap_or("");
                    assert_eq!(detail_kind, "pass", "expected pass, got body: {body}");
                }
                reached = true;
                break;
            }
        }
    }
    assert!(reached, "job did not reach terminal; last `{last_state}`");
    assert_eq!(last_state, "done");

    handles.server_task.abort();
    handles.worker_task.abort();
}
