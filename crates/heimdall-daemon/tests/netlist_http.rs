//! Integration test for GET /duts/:id/netlist.svg.

#![cfg(all(feature = "sqlite", feature = "aegis"))]

use std::sync::Arc;

use heimdall_config::{
    ConfigFile, DutCfg, GoldenBackendCfg, GoldenCfg, HostCfg, JtagDriver, JtagTransportCfg,
    PadDirection, SpiceWatchCfg, ToolsCfg, TransportSection,
};
use heimdall_core::DutKind;
use heimdall_daemon::{BlobStore, JobStore, LocalFsBlobStore, SqliteJobStore, runtime};
use tempfile::TempDir;

const RC_DIVIDER: &str = "* test netlist\n\
                          R1 in mid 1k\n\
                          R2 mid 0 1k\n\
                          C1 mid 0 1n\n\
                          .end\n";

async fn start_with_netlist(
    netlist_path: std::path::PathBuf,
) -> (heimdall_daemon::DaemonHandles, TempDir) {
    let tmp = TempDir::new().expect("tmp");
    let store = SqliteJobStore::open_in_memory().await.expect("store");
    let blobs = LocalFsBlobStore::open(tmp.path().to_path_buf())
        .await
        .expect("blobs");

    let cfg = ConfigFile {
        host: HostCfg {
            name: "rig-spice".into(),
            bind: "127.0.0.1:0".parse().unwrap(),
        },
        duts: vec![
            DutCfg {
                id: "spice-1".into(),
                kind: DutKind::AegisLuna1,
                chip_serial: None,
                transports: vec!["jtag.mock".into()],
                expect_idcode: None,
                bringup: None,
                netlist: Some(netlist_path),
                spice_watches: vec![
                    SpiceWatchCfg {
                        name: "io_in".into(),
                        spice_node: "in".into(),
                        direction: PadDirection::In,
                    },
                    SpiceWatchCfg {
                        name: "io_mid".into(),
                        spice_node: "mid".into(),
                        direction: PadDirection::Out,
                    },
                ],
            },
            DutCfg {
                id: "spice-none".into(),
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
async fn netlist_svg_renders_for_configured_dut() {
    let tmp = TempDir::new().expect("tmp");
    let nl = tmp.path().join("rc.sp");
    std::fs::write(&nl, RC_DIVIDER).unwrap();
    let (handles, _t) = start_with_netlist(nl).await;

    let url = format!("http://{}/duts/spice-1/netlist.svg", handles.local_addr);
    let resp = reqwest::get(&url).await.expect("get");
    assert_eq!(resp.status(), 200);
    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(ct.starts_with("image/svg+xml"), "got content-type {ct}");

    let body = resp.text().await.unwrap();
    assert!(body.starts_with("<svg"), "body should be SVG, got: {body}");
    assert!(body.contains(">r1<"));
    assert!(body.contains(">in<"));
    assert!(body.contains(">mid<"));
    // 'in' should be classified as input (blue); 'mid' as watched-cold (amber).
    assert!(body.contains("#7aa2f7"), "expected input blue");
    assert!(body.contains("#e0af68"), "expected watched-cold amber");

    handles.server_task.abort();
    handles.worker_task.abort();
}

#[tokio::test]
async fn netlist_svg_404_for_dut_without_netlist() {
    let tmp = TempDir::new().expect("tmp");
    let nl = tmp.path().join("rc.sp");
    std::fs::write(&nl, RC_DIVIDER).unwrap();
    let (handles, _t) = start_with_netlist(nl).await;

    let url = format!("http://{}/duts/spice-none/netlist.svg", handles.local_addr);
    let resp = reqwest::get(&url).await.expect("get");
    assert_eq!(resp.status(), 404);

    handles.server_task.abort();
    handles.worker_task.abort();
}

#[tokio::test]
async fn netlist_svg_404_for_unknown_dut() {
    let tmp = TempDir::new().expect("tmp");
    let nl = tmp.path().join("rc.sp");
    std::fs::write(&nl, RC_DIVIDER).unwrap();
    let (handles, _t) = start_with_netlist(nl).await;

    let url = format!("http://{}/duts/no-such/netlist.svg", handles.local_addr);
    let resp = reqwest::get(&url).await.expect("get");
    assert_eq!(resp.status(), 404);

    handles.server_task.abort();
    handles.worker_task.abort();
}

#[tokio::test]
async fn missing_netlist_file_rejected_at_startup() {
    let tmp = TempDir::new().expect("tmp");
    let store = SqliteJobStore::open_in_memory().await.expect("store");
    let blobs = LocalFsBlobStore::open(tmp.path().to_path_buf())
        .await
        .expect("blobs");
    let cfg = ConfigFile {
        host: HostCfg {
            name: "rig".into(),
            bind: "127.0.0.1:0".parse().unwrap(),
        },
        duts: vec![DutCfg {
            id: "spice-1".into(),
            kind: DutKind::AegisLuna1,
            chip_serial: None,
            transports: vec!["jtag.mock".into()],
            expect_idcode: None,
            bringup: None,
            netlist: Some(std::path::PathBuf::from("/nonexistent.sp")),
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
    let result = runtime::start_with_config(
        bind,
        Arc::new(store) as Arc<dyn JobStore>,
        Arc::new(blobs) as Arc<dyn BlobStore>,
        &cfg,
    )
    .await;
    let err = match result {
        Ok(_) => panic!("expected startup failure for missing netlist"),
        Err(e) => e,
    };
    let msg = format!("{err}");
    assert!(
        msg.contains("netlist") && msg.contains("does not exist"),
        "got: {msg}"
    );
}
