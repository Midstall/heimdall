//! GET /duts must surface the resolved effective ISA per DUT so the
//! web UI can render the XLEN + extension chips strip. Verifies the
//! `effective_isa` envelope shape and the "config" source label
//! when no JTAG probe has run yet.

#![cfg(feature = "sqlite")]

use std::sync::Arc;

use heimdall_core::DutKind;
use heimdall_daemon::{
    BlobStore, DutRecord, DutRegistry, IsaSpec, JobStore, LocalFsBlobStore, SqliteJobStore, runtime,
};
use tempfile::TempDir;

#[tokio::test]
async fn duts_route_includes_effective_isa_when_config_sets_one() {
    let tmp = TempDir::new().expect("tmp");
    let store = SqliteJobStore::open_in_memory().await.expect("store");
    let blobs = LocalFsBlobStore::open(tmp.path().to_path_buf())
        .await
        .expect("blobs");

    let mut registry = DutRegistry::new();
    let mut rec = DutRecord::mock("river-1", DutKind::RiverRc1Nano);
    rec.isa = Some(IsaSpec {
        xlen: 64,
        extensions: vec!["i".into(), "m".into(), "a".into(), "c".into()],
    });
    registry.insert(rec);

    let bind: std::net::SocketAddr = "127.0.0.1:0".parse().unwrap();
    let handles = runtime::start_with_dut_registry(
        bind,
        Arc::new(store) as Arc<dyn JobStore>,
        Arc::new(blobs) as Arc<dyn BlobStore>,
        Arc::new(registry),
    )
    .await
    .expect("daemon start");

    let url = format!("http://{}/duts", handles.local_addr);
    let body: serde_json::Value = reqwest::get(&url)
        .await
        .expect("get")
        .json()
        .await
        .expect("json");
    let duts = body["duts"].as_array().expect("duts array");
    assert_eq!(duts.len(), 1, "exactly one configured DUT");
    let isa = &duts[0]["effective_isa"];
    assert!(
        !isa.is_null(),
        "effective_isa must be present when config has one"
    );
    assert_eq!(isa["xlen"], 64, "xlen surfaces verbatim from config");
    assert_eq!(
        isa["source"], "config",
        "source must be `config` when no JTAG probe has run"
    );
    let exts = isa["extensions"].as_array().expect("extensions array");
    let names: Vec<String> = exts
        .iter()
        .map(|v| v.as_str().unwrap_or("").to_string())
        .collect();
    assert_eq!(
        names,
        vec!["i".to_string(), "m".into(), "a".into(), "c".into()],
        "extensions must round-trip in declared order"
    );

    handles.server_task.abort();
    handles.worker_task.abort();
}

/// When no isa spec is set on the DUT and no probe has run, the
/// `effective_isa` field is absent (skip_serializing_if) so the UI
/// renders "unknown" instead of lying with a default.
#[tokio::test]
async fn duts_route_omits_effective_isa_when_unresolved() {
    let tmp = TempDir::new().expect("tmp");
    let store = SqliteJobStore::open_in_memory().await.expect("store");
    let blobs = LocalFsBlobStore::open(tmp.path().to_path_buf())
        .await
        .expect("blobs");

    let mut registry = DutRegistry::new();
    registry.insert(DutRecord::mock("noisa-1", DutKind::RiverRc1Nano));
    let bind: std::net::SocketAddr = "127.0.0.1:0".parse().unwrap();
    let handles = runtime::start_with_dut_registry(
        bind,
        Arc::new(store) as Arc<dyn JobStore>,
        Arc::new(blobs) as Arc<dyn BlobStore>,
        Arc::new(registry),
    )
    .await
    .expect("daemon start");

    let url = format!("http://{}/duts", handles.local_addr);
    let body: serde_json::Value = reqwest::get(&url)
        .await
        .expect("get")
        .json()
        .await
        .expect("json");
    assert!(
        body["duts"][0]["effective_isa"].is_null()
            || body["duts"][0].get("effective_isa").is_none(),
        "effective_isa must be absent when no spec or probe is available"
    );

    handles.server_task.abort();
    handles.worker_task.abort();
}
