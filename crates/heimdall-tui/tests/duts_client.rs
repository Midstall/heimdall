//! Integration test: TUI client talks to a real daemon's `/duts` endpoint
//! and produces the rows the DUT view expects.

#![cfg(feature = "sqlite")]

use std::sync::Arc;

use heimdall_config::{
    ConfigFile, DutCfg, GoldenBackendCfg, GoldenCfg, HostCfg, JtagDriver, JtagTransportCfg,
    ToolsCfg, TransportSection,
};
use heimdall_core::DutKind;
use heimdall_daemon::{BlobStore, JobStore, LocalFsBlobStore, SqliteJobStore, runtime};
use heimdall_tui::DaemonClient;
use tempfile::TempDir;

async fn start_with_two_duts() -> (heimdall_daemon::DaemonHandles, TempDir) {
    let tmp = TempDir::new().expect("tmp");
    let store = SqliteJobStore::open_in_memory().await.expect("store");
    let blobs = LocalFsBlobStore::open(tmp.path().to_path_buf())
        .await
        .expect("blobs");

    let jtag_mock = JtagTransportCfg {
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
    };

    let cfg = ConfigFile {
        host: HostCfg {
            name: "rig-tui".into(),
            bind: "127.0.0.1:0".parse().unwrap(),
        },
        duts: vec![
            DutCfg {
                id: "river-1".into(),
                kind: DutKind::RiverRc1Nano,
                chip_serial: Some("SN-001".into()),
                transports: vec!["jtag.mock".into()],
                expect_idcode: None,
                bringup: None,
                netlist: None,
                spice_watches: vec![],
            },
            DutCfg {
                id: "aegis-1".into(),
                kind: DutKind::AegisLuna1,
                chip_serial: None,
                transports: vec!["jtag.mock".into()],
                expect_idcode: None,
                bringup: None,
                netlist: None,
                spice_watches: vec![],
            },
        ],
        transport: TransportSection {
            jtag: vec![jtag_mock],
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
async fn list_duts_returns_configured_rows() {
    let (handles, _tmp) = start_with_two_duts().await;
    let client = DaemonClient::new(format!("http://{}", handles.local_addr));

    let rows = client.list_duts().await.expect("list_duts");
    assert_eq!(rows.len(), 2, "expected 2 DUTs, got: {rows:?}");

    let by_id: std::collections::HashMap<_, _> = rows.iter().map(|d| (d.id.clone(), d)).collect();

    let river = by_id.get("river-1").expect("river-1 row missing");
    assert_eq!(river.kind, "river-rc1-nano");
    assert_eq!(river.chip_serial.as_deref(), Some("SN-001"));
    assert_eq!(river.jtag_driver.as_deref(), Some("mock"));
    assert!(river.leased_by.is_none());
    // Mock transport: probe returns Unknown.
    assert_eq!(
        river.connection_status,
        heimdall_tui::ConnectionStatus::Unknown
    );

    let aegis = by_id.get("aegis-1").expect("aegis-1 row missing");
    assert_eq!(aegis.kind, "aegis-luna1");
    assert!(aegis.chip_serial.is_none());
    assert_eq!(aegis.jtag_driver.as_deref(), Some("mock"));
    assert_eq!(
        aegis.connection_status,
        heimdall_tui::ConnectionStatus::Unknown
    );

    handles.server_task.abort();
    handles.worker_task.abort();
}

#[tokio::test]
async fn list_duts_handles_empty_registry() {
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

    let client = DaemonClient::new(format!("http://{}", handles.local_addr));
    let rows = client.list_duts().await.expect("list_duts");
    assert!(rows.is_empty());

    handles.server_task.abort();
    handles.worker_task.abort();
}
