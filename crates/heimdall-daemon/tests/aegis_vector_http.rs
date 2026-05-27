//! HTTP integration test for the RunAegisVector JobKind: load bitstream, drive
//! pads, observe outputs, diff against expected. Uses Mock GPIO so the outputs
//! are deterministic (MockTransport::read returns false on empty queue).

#![cfg(all(feature = "sqlite", feature = "aegis"))]

use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use heimdall_config::{
    ConfigFile, DutCfg, GoldenBackendCfg, GoldenCfg, GpioDriver, GpioTransportCfg, HostCfg,
    JtagDriver, JtagTransportCfg, PadDirection, PadMapEntry, ToolsCfg, TransportSection,
};
use heimdall_core::DutKind;
use heimdall_daemon::{BlobStore, JobStore, LocalFsBlobStore, SqliteJobStore, runtime};
use serde_json::json;
use tempfile::TempDir;

async fn start_with_pinmap() -> (heimdall_daemon::DaemonHandles, TempDir) {
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
        duts: vec![DutCfg {
            id: "luna1-1".into(),
            kind: DutKind::AegisLuna1,
            chip_serial: Some("L1-0001".into()),
            transports: vec!["jtag.mock".into(), "gpio.host".into()],
            expect_idcode: None,
            bringup: None,
            netlist: None,
            spice_watches: vec![],
        }],
        transport: TransportSection {
            jtag: vec![JtagTransportCfg {
                id: "jtag.mock".into(),
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
            }],
            gpio: vec![GpioTransportCfg {
                id: "gpio.host".into(),
                driver: GpioDriver::Mock,
                device: None,
            }],
            ..Default::default()
        },
        golden: GoldenCfg {
            aegis: Some(GoldenBackendCfg::Mock),
            river: None,
        },
        tools: ToolsCfg::default(),
        pad_maps: vec![
            PadMapEntry {
                dut: "luna1-1".into(),
                direction: PadDirection::In,
                fpga_pad: 0,
                gpio_line: 10,
                gpio_transport: "gpio.host".into(),
            },
            PadMapEntry {
                dut: "luna1-1".into(),
                direction: PadDirection::In,
                fpga_pad: 1,
                gpio_line: 11,
                gpio_transport: "gpio.host".into(),
            },
            PadMapEntry {
                dut: "luna1-1".into(),
                direction: PadDirection::Out,
                fpga_pad: 2,
                gpio_line: 12,
                gpio_transport: "gpio.host".into(),
            },
            PadMapEntry {
                dut: "luna1-1".into(),
                direction: PadDirection::Out,
                fpga_pad: 3,
                gpio_line: 13,
                gpio_transport: "gpio.host".into(),
            },
        ],
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

fn descriptor_json() -> &'static str {
    r#"{
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
    }"#
}

#[tokio::test]
async fn run_aegis_vector_completes_with_pass() {
    let (handles, _tmp) = start_with_pinmap().await;
    let client = reqwest::Client::new();

    let bitstream = vec![0u8; (233u32).div_ceil(8) as usize];
    let bitstream_b64 = base64::engine::general_purpose::STANDARD.encode(&bitstream);

    // MockTransport::read returns false on empty queue, so expected outputs
    // both default to false. The test asserts Done(Pass).
    let body = json!({
        "dut": "luna1-1",
        "kind": {
            "kind": "run-aegis-vector",
            "descriptor_json": descriptor_json(),
            "bitstream_b64": bitstream_b64,
            "inputs": { "io_0": true, "io_1": false },
            "expected_outputs": { "io_2": false, "io_3": false },
            "settle_cycles": 1,
        }
    });
    let url = format!("http://{}/jobs", handles.local_addr);
    let resp = client.post(&url).json(&body).send().await.expect("post");
    assert_eq!(resp.status(), 201, "expected 201; got {}", resp.status());
    let job: serde_json::Value = resp.json().await.expect("json");
    let job_id = job["id"].as_str().expect("id").to_string();

    // Poll for terminal state.
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
                    assert_eq!(detail_kind, "pass", "expected pass, got body: {body}");
                }
                reached = true;
                break;
            }
        }
    }
    assert!(reached, "job never reached terminal; last `{last_state}`");
    assert_eq!(last_state, "done");

    handles.server_task.abort();
    handles.worker_task.abort();
}

#[tokio::test]
async fn run_aegis_vector_fails_on_wrong_expected_outputs() {
    let (handles, _tmp) = start_with_pinmap().await;
    let client = reqwest::Client::new();

    let bitstream = vec![0u8; (233u32).div_ceil(8) as usize];
    let bitstream_b64 = base64::engine::general_purpose::STANDARD.encode(&bitstream);

    // We expect io_2 = true. The mock returns false, so the diff should fail.
    let body = json!({
        "dut": "luna1-1",
        "kind": {
            "kind": "run-aegis-vector",
            "descriptor_json": descriptor_json(),
            "bitstream_b64": bitstream_b64,
            "inputs": {},
            "expected_outputs": { "io_2": true },
            "settle_cycles": 1,
        }
    });
    let url = format!("http://{}/jobs", handles.local_addr);
    let resp = client.post(&url).json(&body).send().await.expect("post");
    assert_eq!(resp.status(), 201);
    let job: serde_json::Value = resp.json().await.expect("json");
    let job_id = job["id"].as_str().unwrap().to_string();

    let url = format!("http://{}/jobs/{job_id}", handles.local_addr);
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut reached_fail = false;
    let mut last_body = serde_json::Value::Null;
    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let resp = client.get(&url).send().await.expect("get");
        if resp.status() == 200 {
            let body: serde_json::Value = resp.json().await.expect("json");
            let state = body["state"]["state"].as_str().unwrap_or("");
            if state == "done" {
                let kind = body["state"]["detail"]["kind"].as_str().unwrap_or("");
                if kind == "fail" {
                    reached_fail = true;
                    last_body = body;
                    break;
                }
            }
        }
    }
    assert!(
        reached_fail,
        "expected Done(Fail) verdict on output mismatch; last body: {last_body}"
    );
    handles.server_task.abort();
    handles.worker_task.abort();
}
