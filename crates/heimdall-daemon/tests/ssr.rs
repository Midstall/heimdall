//! Server-side rendering tests for the `/` route. Verifies that current
//! jobs/campaigns/DUTs and the requested locale are baked into the initial
//! HTML response (no JS needed for first paint).

#![cfg(feature = "sqlite")]

use std::sync::Arc;

use heimdall_config::{
    ConfigFile, DutCfg, GoldenBackendCfg, GoldenCfg, HostCfg, JtagDriver, JtagTransportCfg,
    ToolsCfg, TransportSection,
};
use heimdall_core::{DutId, DutKind};
use heimdall_daemon::{
    BlobStore, JobKind, JobStore, LocalFsBlobStore, NewJob, SqliteJobStore, runtime,
};
use tempfile::TempDir;

async fn start_with_one_dut() -> (heimdall_daemon::DaemonHandles, TempDir) {
    let tmp = TempDir::new().expect("tmp");
    let store = SqliteJobStore::open_in_memory().await.expect("store");
    let blobs = LocalFsBlobStore::open(tmp.path().to_path_buf())
        .await
        .expect("blobs");

    let cfg = ConfigFile {
        host: HostCfg {
            name: "rig-ssr".into(),
            bind: "127.0.0.1:0".parse().unwrap(),
        },
        duts: vec![DutCfg {
            id: "river-ssr-1".into(),
            kind: DutKind::RiverRc1Nano,
            chip_serial: Some("SN-SSR".into()),
            transports: vec!["jtag.mock".into()],
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
async fn ssr_bakes_dut_state_into_html() {
    let (handles, _tmp) = start_with_one_dut().await;
    let url = format!("http://{}/", handles.local_addr);
    let body = reqwest::get(&url).await.unwrap().text().await.unwrap();

    // The DUT card should be present in the initial HTML (no JS fetch needed).
    assert!(
        body.contains("river-ssr-1"),
        "expected SSR'd DUT id, got:\n{body}"
    );
    assert!(body.contains("SN-SSR"), "expected SSR'd chip serial");
    assert!(
        body.contains("dut-status-idle"),
        "mock transport reads as idle"
    );
    // English locale -> "idle" label.
    assert!(body.contains(">idle<"));

    handles.server_task.abort();
    handles.worker_task.abort();
}

#[tokio::test]
async fn ssr_japanese_locale_translates_labels_and_status() {
    let (handles, _tmp) = start_with_one_dut().await;
    let url = format!("http://{}/?lang=ja", handles.local_addr);
    let body = reqwest::get(&url).await.unwrap().text().await.unwrap();

    // Tab labels render in Japanese.
    assert!(
        body.contains("ジョブ"),
        "expected ja tab label 'ジョブ':\n{body}"
    );
    assert!(body.contains("キャンペーン"));
    // Status label for unknown is "アイドル" in Japanese.
    assert!(
        body.contains("アイドル"),
        "expected ja status 'アイドル' baked into SSR"
    );
    // html lang attribute reflects the locale.
    assert!(body.contains("lang=\"ja\""));

    handles.server_task.abort();
    handles.worker_task.abort();
}

#[tokio::test]
async fn ssr_accept_language_header_picks_locale() {
    let (handles, _tmp) = start_with_one_dut().await;
    let url = format!("http://{}/", handles.local_addr);
    let body = reqwest::Client::new()
        .get(&url)
        .header("Accept-Language", "ja, en;q=0.5")
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(body.contains("lang=\"ja\""));
    assert!(body.contains("ジョブ"));
    handles.server_task.abort();
    handles.worker_task.abort();
}

#[tokio::test]
async fn ssr_renders_empty_state_when_no_data() {
    let tmp = TempDir::new().unwrap();
    let store = SqliteJobStore::open_in_memory().await.unwrap();
    let blobs = LocalFsBlobStore::open(tmp.path().to_path_buf())
        .await
        .unwrap();
    let bind: std::net::SocketAddr = "127.0.0.1:0".parse().unwrap();
    let handles = runtime::start(
        bind,
        Arc::new(store) as Arc<dyn JobStore>,
        Arc::new(blobs) as Arc<dyn BlobStore>,
    )
    .await
    .unwrap();

    let url = format!("http://{}/", handles.local_addr);
    let body = reqwest::get(&url).await.unwrap().text().await.unwrap();
    assert!(body.contains("no jobs"));
    assert!(body.contains("no campaigns"));
    assert!(body.contains("no DUTs configured"));
    handles.server_task.abort();
    handles.worker_task.abort();
}

#[tokio::test]
async fn ssr_bakes_jobs_into_table_rows() {
    let (handles, _tmp) = start_with_one_dut().await;
    // Insert a job directly via the daemon's API.
    let url_jobs = format!("http://{}/jobs", handles.local_addr);
    let body = serde_json::to_string(&NewJob {
        dut: DutId::new("river-ssr-1"),
        kind: JobKind::MockHello,
        campaign: None,
    })
    .unwrap();
    let resp = reqwest::Client::new()
        .post(&url_jobs)
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());

    let html = reqwest::get(format!("http://{}/", handles.local_addr))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        html.contains("river-ssr-1"),
        "job row should reference its DUT id"
    );
    assert!(
        html.contains("mock-hello"),
        "job kind should be present in SSR HTML"
    );
    handles.server_task.abort();
    handles.worker_task.abort();
}
