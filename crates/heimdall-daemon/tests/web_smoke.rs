//! Smoke test for the embedded web UI. Verifies GET / returns the index HTML
//! and GET /assets/{app.css,app.js} return the corresponding files.

#![cfg(feature = "sqlite")]

use std::sync::Arc;

use heimdall_daemon::{BlobStore, JobStore, LocalFsBlobStore, SqliteJobStore, runtime};
use tempfile::TempDir;

async fn start_daemon() -> (heimdall_daemon::DaemonHandles, TempDir) {
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
async fn index_returns_html() {
    let (handles, _tmp) = start_daemon().await;
    let url = format!("http://{}/", handles.local_addr);
    let resp = reqwest::get(&url).await.expect("get");
    assert_eq!(resp.status(), 200);
    let ct = resp
        .headers()
        .get("content-type")
        .map(|v| v.to_str().unwrap_or(""))
        .unwrap_or("");
    assert!(
        ct.contains("html"),
        "expected html content-type, got `{ct}`"
    );
    let body = resp.text().await.expect("text");
    assert!(body.contains("<title") && body.contains(">Heimdall</title>"));
    assert!(body.contains("HEIMDALL"));
    handles.server_task.abort();
    handles.worker_task.abort();
}

#[tokio::test]
async fn css_contains_tokyo_night_palette() {
    let (handles, _tmp) = start_daemon().await;
    let url = format!("http://{}/assets/app.css", handles.local_addr);
    let resp = reqwest::get(&url).await.expect("get");
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.expect("text");
    // Tokyo Night background.
    assert!(body.contains("#1a1b26"), "expected base bg color in css");
    // Verdict color semantics.
    assert!(body.contains("--verdict-pass"));
    handles.server_task.abort();
    handles.worker_task.abort();
}

#[tokio::test]
async fn js_is_served() {
    let (handles, _tmp) = start_daemon().await;
    let url = format!("http://{}/assets/app.js", handles.local_addr);
    let resp = reqwest::get(&url).await.expect("get");
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.expect("text");
    assert!(body.contains("/events"), "js should subscribe to /events");
    handles.server_task.abort();
    handles.worker_task.abort();
}

#[tokio::test]
async fn index_includes_duts_view() {
    let (handles, _tmp) = start_daemon().await;
    let url = format!("http://{}/", handles.local_addr);
    let resp = reqwest::get(&url).await.expect("get");
    let body = resp.text().await.expect("text");
    assert!(
        body.contains("data-view=\"duts\""),
        "DUTs tab missing from index.html"
    );
    assert!(
        body.contains("id=\"view-duts\""),
        "DUTs view section missing"
    );
    handles.server_task.abort();
    handles.worker_task.abort();
}

#[tokio::test]
async fn js_fetches_duts() {
    let (handles, _tmp) = start_daemon().await;
    let url = format!("http://{}/assets/app.js", handles.local_addr);
    let resp = reqwest::get(&url).await.expect("get");
    let body = resp.text().await.expect("text");
    assert!(body.contains("refreshDuts"));
    assert!(body.contains("netlist-panel"));
    handles.server_task.abort();
    handles.worker_task.abort();
}

#[tokio::test]
async fn html_marks_translatable_elements_with_data_i18n() {
    let (handles, _tmp) = start_daemon().await;
    let url = format!("http://{}/", handles.local_addr);
    let body = reqwest::get(&url).await.unwrap().text().await.unwrap();
    // Every visible label must carry a data-i18n attribute so the JS can
    // localize it at boot. Spot-check the tab labels and a column header.
    assert!(body.contains("data-i18n=\"web.tabs.jobs\""));
    assert!(body.contains("data-i18n=\"web.tabs.campaigns\""));
    assert!(body.contains("data-i18n=\"web.tabs.duts\""));
    assert!(body.contains("data-i18n=\"web.duts.no_duts\""));
    handles.server_task.abort();
    handles.worker_task.abort();
}

#[tokio::test]
async fn js_fetches_and_applies_i18n_catalog() {
    let (handles, _tmp) = start_daemon().await;
    let url = format!("http://{}/assets/app.js", handles.local_addr);
    let body = reqwest::get(&url).await.unwrap().text().await.unwrap();
    assert!(body.contains("/i18n.json"), "JS should fetch /i18n.json");
    assert!(
        body.contains("data-i18n"),
        "JS should walk [data-i18n] elements"
    );
    assert!(body.contains("initI18n"), "JS should expose initI18n()");
    handles.server_task.abort();
    handles.worker_task.abort();
}

#[tokio::test]
async fn js_renders_connection_status_badge() {
    let (handles, _tmp) = start_daemon().await;
    let js_url = format!("http://{}/assets/app.js", handles.local_addr);
    let css_url = format!("http://{}/assets/app.css", handles.local_addr);
    let js = reqwest::get(&js_url).await.unwrap().text().await.unwrap();
    let css = reqwest::get(&css_url).await.unwrap().text().await.unwrap();
    // JS must read the field and render a status pill class.
    assert!(
        js.contains("connection_status"),
        "JS should read d.connection_status"
    );
    assert!(
        js.contains("dut-status-"),
        "JS should emit a dut-status-<state> class"
    );
    // CSS must style all three states.
    assert!(css.contains(".dut-status-connected"));
    assert!(css.contains(".dut-status-disconnected"));
    assert!(css.contains(".dut-status-idle"));
    handles.server_task.abort();
    handles.worker_task.abort();
}

#[tokio::test]
async fn favicon_svg_is_served_and_referenced_by_index() {
    let (handles, _tmp) = start_daemon().await;
    // The favicon is served from /assets/favicon.svg.
    let url = format!("http://{}/assets/favicon.svg", handles.local_addr);
    let resp = reqwest::get(&url).await.expect("get");
    assert_eq!(resp.status(), 200);
    let ct = resp
        .headers()
        .get("content-type")
        .map(|v| v.to_str().unwrap_or(""))
        .unwrap_or("");
    assert!(ct.contains("svg"), "expected svg content-type, got `{ct}`");
    let body = resp.text().await.expect("text");
    assert!(body.starts_with("<?xml"), "favicon should be a valid SVG");
    assert!(body.contains("<svg"));

    // The SSR'd index references it as <link rel="icon">.
    let index = reqwest::get(format!("http://{}/", handles.local_addr))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        index.contains(r#"href="/assets/favicon.svg""#),
        "<link rel=icon> missing from SSR HTML"
    );
    assert!(index.contains(r#"rel="icon""#));

    handles.server_task.abort();
    handles.worker_task.abort();
}

#[tokio::test]
async fn missing_asset_returns_404() {
    let (handles, _tmp) = start_daemon().await;
    let url = format!("http://{}/assets/does-not-exist.txt", handles.local_addr);
    let resp = reqwest::get(&url).await.expect("get");
    assert_eq!(resp.status(), 404);
    handles.server_task.abort();
    handles.worker_task.abort();
}
