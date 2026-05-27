//! Integration test: River BootRiverElf with OpenocdSpawn transport. Uses
//! `sleep` as the fake OpenOCD binary so the Tcl port never binds and the
//! transport open times out. The worker must report Failed.

#![cfg(all(feature = "sqlite", feature = "river"))]

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
async fn river_openocd_spawn_failure_surfaces_as_job_failed() {
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
            id: "river-1".into(),
            kind: DutKind::RiverRc1Nano,
            chip_serial: Some("R1N-0001".into()),
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
                openocd_endpoint: Some("127.0.0.1:55325".parse().unwrap()),
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
            river: Some(GoldenBackendCfg::Mock),
            aegis: None,
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
    let fake_elf = b"\x7fELF\x02\x01\x01\x00";
    let elf_b64 = base64::engine::general_purpose::STANDARD.encode(fake_elf);
    let body = json!({
        "dut": "river-1",
        "kind": {"kind": "boot-river-elf", "elf_b64": elf_b64, "cycles": 1000u64},
    });
    let url = format!("http://{}/jobs", handles.local_addr);
    let resp = client.post(&url).json(&body).send().await.expect("post");
    assert_eq!(resp.status(), 201);
    let job: serde_json::Value = resp.json().await.expect("json");
    let job_id = job["id"].as_str().unwrap().to_string();

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
        "expected Failed (spawned openocd times out); got `{last_state}`"
    );

    handles.server_task.abort();
    handles.worker_task.abort();
}
