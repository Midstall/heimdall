//! Integration test for GET /i18n.json: locale resolution by query string,
//! Accept-Language header, and default fallback.

#![cfg(feature = "sqlite")]

use std::sync::Arc;

use heimdall_daemon::{BlobStore, JobStore, LocalFsBlobStore, SqliteJobStore, runtime};
use tempfile::TempDir;

async fn start() -> (heimdall_daemon::DaemonHandles, TempDir) {
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

#[tokio::test]
async fn i18n_defaults_to_english() {
    let (handles, _tmp) = start().await;
    let url = format!("http://{}/i18n.json", handles.local_addr);
    let body: serde_json::Value = reqwest::get(&url).await.unwrap().json().await.unwrap();
    assert_eq!(body["_locale"], "en");
    assert_eq!(body["tui.view.jobs"], "jobs");
    assert_eq!(body["common.status.connected"], "connected");
    handles.server_task.abort();
    handles.worker_task.abort();
}

#[tokio::test]
async fn i18n_query_param_selects_japanese() {
    let (handles, _tmp) = start().await;
    let url = format!("http://{}/i18n.json?lang=ja", handles.local_addr);
    let body: serde_json::Value = reqwest::get(&url).await.unwrap().json().await.unwrap();
    assert_eq!(body["_locale"], "ja");
    assert_eq!(body["tui.view.jobs"], "ジョブ");
    assert_eq!(body["common.status.connected"], "接続済み");
    handles.server_task.abort();
    handles.worker_task.abort();
}

#[tokio::test]
async fn i18n_accept_language_header_picks_japanese() {
    let (handles, _tmp) = start().await;
    let url = format!("http://{}/i18n.json", handles.local_addr);
    let client = reqwest::Client::new();
    let body: serde_json::Value = client
        .get(&url)
        .header("Accept-Language", "ja, en;q=0.5")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(body["_locale"], "ja");
    assert_eq!(body["tui.duts.col_serial"], "シリアル");
    handles.server_task.abort();
    handles.worker_task.abort();
}

#[tokio::test]
async fn i18n_query_param_beats_accept_language() {
    let (handles, _tmp) = start().await;
    let url = format!("http://{}/i18n.json?lang=en", handles.local_addr);
    let client = reqwest::Client::new();
    let body: serde_json::Value = client
        .get(&url)
        .header("Accept-Language", "ja")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(body["_locale"], "en");
    assert_eq!(body["tui.view.jobs"], "jobs");
    handles.server_task.abort();
    handles.worker_task.abort();
}

#[tokio::test]
async fn i18n_unknown_lang_falls_back_to_english() {
    let (handles, _tmp) = start().await;
    let url = format!("http://{}/i18n.json?lang=klingon", handles.local_addr);
    let body: serde_json::Value = reqwest::get(&url).await.unwrap().json().await.unwrap();
    assert_eq!(body["_locale"], "en");
    handles.server_task.abort();
    handles.worker_task.abort();
}
