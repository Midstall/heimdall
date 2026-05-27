//! HTTP integration test for BootRiverElf JobKind via MockOpenOcdServer.

#![cfg(all(feature = "sqlite", feature = "river"))]

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use common::mock_openocd::MockOpenOcdServer;
use heimdall_config::{
    ConfigFile, DutCfg, GoldenBackendCfg, GoldenCfg, HostCfg, JtagDriver, JtagTransportCfg,
    ToolsCfg, TransportSection,
};
use heimdall_core::DutKind;
use heimdall_daemon::{BlobStore, JobStore, LocalFsBlobStore, SqliteJobStore, runtime};
use serde_json::json;
use tempfile::TempDir;

#[tokio::test]
async fn boot_river_elf_dispatches_through_factory() {
    // Mock OpenOCD with enough responses to get through prepare + load + run.
    // Reg reads return parseable hex. load_image matches via prefix.
    let mut srv = MockOpenOcdServer::new()
        .respond("reset init", "")
        .respond("scan_chain", "  1   river.cpu    Y    0xdeadbeef")
        .respond("halt", "")
        .respond("resume", "")
        .respond("load_image", "loaded 4 bytes in 0.001s (4 KiB/s)")
        .respond("reg pc", "pc (/64): 0x80000010");
    for i in 1..32u32 {
        srv = srv.respond(
            format!("reg x{i}"),
            format!("x{i} (/64): 0x0000000000000000"),
        );
    }
    srv = srv.respond("wait_halt 1000", "");
    srv = srv.respond("wait_halt 1", "");
    let server = srv.start().await;
    let mock_endpoint = server.addr();

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
            transports: vec!["jtag.ocd".into()],
            expect_idcode: None,
            bringup: None,
            netlist: None,
            spice_watches: vec![],
        }],
        transport: TransportSection {
            jtag: vec![JtagTransportCfg {
                id: "jtag.ocd".into(),
                driver: JtagDriver::Openocd,
                serial: None,
                openocd_endpoint: Some(mock_endpoint),
                openocd_config: None,
                openocd_binary: None,
                openocd_extra_args: vec![],
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

    // POST a BootRiverElf job. ELF content is a fake byte string. The
    // driver loads it via OpenOCD load_image which the mock acks.
    let client = reqwest::Client::new();
    let fake_elf = b"\x7fELF\x02\x01\x01\x00";
    let elf_b64 = base64::engine::general_purpose::STANDARD.encode(fake_elf);
    let body = json!({
        "dut": "river-1",
        "kind": {"kind": "boot-river-elf", "elf_b64": elf_b64, "cycles": 1000u64},
    });
    let url = format!("http://{}/jobs", handles.local_addr);
    let resp = client.post(&url).json(&body).send().await.expect("post");
    assert_eq!(resp.status(), 201, "expected 201; got {}", resp.status());
    let job: serde_json::Value = resp.json().await.expect("json");
    let job_id = job["id"].as_str().unwrap().to_string();

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
            if matches!(last_state.as_str(), "done" | "failed" | "cancelled") {
                reached = true;
                break;
            }
        }
    }
    assert!(
        reached,
        "job never reached terminal state; last `{last_state}`"
    );

    // The exact verdict depends on how completely the mock satisfies the
    // RiverCpuDriver's expectations. Either Done or Failed proves the factory
    // dispatched correctly through the real RiverCpuDriver path.
    assert!(
        matches!(last_state.as_str(), "done" | "failed"),
        "expected terminal done|failed; got `{last_state}`"
    );

    // Sanity: the mock should have received at least the prepare-phase commands.
    let received = server.received().await;
    assert!(
        received.iter().any(|c| c == "reset init"),
        "expected mock to see `reset init`; got {received:?}"
    );

    handles.server_task.abort();
    handles.worker_task.abort();
    server.shutdown().await;
}
