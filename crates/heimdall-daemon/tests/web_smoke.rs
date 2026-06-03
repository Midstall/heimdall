//! Smoke test for the embedded web UI. Verifies GET / returns the
//! index HTML and the bundled assets are served at /assets/.
//!
//! The frontend ships as a single rolldown-bundled, minified
//! `app.js`. Content-grep tests pull that one file. The minifier
//! mangles identifiers and strips comments, so substring matches
//! that survive minification need to be either string literals
//! (URLs, CSS class names, event-kind tags) or call-site shapes
//! the runtime preserves. Identifier-name greps from the earlier
//! per-module era no longer work and have been retired here.

#![cfg(feature = "sqlite")]

use std::sync::Arc;

use heimdall_daemon::{BlobStore, JobStore, LocalFsBlobStore, SqliteJobStore, runtime};
use tempfile::TempDir;

/// Fetch the bundled frontend JS. Minified, so callers grep for
/// string literals the bundler can't rewrite (URLs, CSS class
/// names, DOM attribute strings).
async fn fetch_bundle_js(addr: std::net::SocketAddr) -> String {
    let url = format!("http://{addr}/assets/app.js");
    let resp = reqwest::get(&url).await.expect("get");
    assert_eq!(
        resp.status(),
        200,
        "bundled app.js must be served from /assets/",
    );
    resp.text().await.expect("text")
}

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

/// The build.rs lucide pipeline must (a) stage `lucide.ttf` so it's
/// served at `/assets/lucide.ttf` and (b) inject @font-face + per-icon
/// classes into app.css so the frontend can resolve `.icon-restart`
/// and friends to glyphs.
#[tokio::test]
async fn lucide_icon_font_and_classes_are_served() {
    let (handles, _tmp) = start_daemon().await;
    let base = format!("http://{}", handles.local_addr);

    let font = reqwest::get(format!("{base}/assets/lucide.ttf"))
        .await
        .expect("get font");
    assert_eq!(font.status(), 200, "ttf must be served");
    let bytes = font.bytes().await.expect("body");
    assert!(
        bytes.len() > 1024,
        "lucide.ttf looks truncated ({} bytes)",
        bytes.len()
    );

    let css = reqwest::get(format!("{base}/assets/app.css"))
        .await
        .expect("get css")
        .text()
        .await
        .expect("text");
    assert!(
        css.contains("@font-face") && css.contains("'Lucide'"),
        "app.css must declare the Lucide @font-face block"
    );
    for class in ["icon-restart", "icon-row-closed", "icon-row-open"] {
        assert!(css.contains(class), "app.css must define .{class} rule");
    }

    handles.server_task.abort();
    handles.worker_task.abort();
}

#[tokio::test]
async fn js_is_served() {
    let (handles, _tmp) = start_daemon().await;
    let js = fetch_bundle_js(handles.local_addr).await;
    assert!(
        js.contains("/events"),
        "frontend bundle should reference /events WebSocket URL",
    );
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
    let js = fetch_bundle_js(handles.local_addr).await;
    // String literals survive minification; we grep for the
    // /duts URL the refreshDuts() helper fetches and the CSS
    // class name the dut card renders into.
    assert!(
        js.contains("/duts"),
        "bundle should reference the /duts API"
    );
    assert!(
        js.contains("netlist-panel"),
        "bundle should reference the .netlist-panel CSS class",
    );
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
    assert!(body.contains("data-i18n=\"web.tabs.about\""));
    assert!(body.contains("data-i18n=\"web.duts.no_duts\""));
    handles.server_task.abort();
    handles.worker_task.abort();
}

/// SSR must include the About tab + an empty `#view-about`
/// section the JS later fills with the `/about` response.
#[tokio::test]
async fn index_includes_about_view() {
    let (handles, _tmp) = start_daemon().await;
    let url = format!("http://{}/", handles.local_addr);
    let body = reqwest::get(&url)
        .await
        .expect("get")
        .text()
        .await
        .expect("text");
    assert!(
        body.contains("data-view=\"about\""),
        "About tab missing from index.html"
    );
    assert!(
        body.contains("id=\"view-about\""),
        "About view section missing"
    );
    assert!(
        body.contains("id=\"about-version\""),
        "About panel missing #about-version slot"
    );
    handles.server_task.abort();
    handles.worker_task.abort();
}

/// The frontend bundle must reference the `/about` endpoint so a
/// click on the About tab triggers the fetch.
#[tokio::test]
async fn bundle_fetches_about_endpoint() {
    let (handles, _tmp) = start_daemon().await;
    let js = fetch_bundle_js(handles.local_addr).await;
    assert!(
        js.contains("/about"),
        "bundle should reference the /about API endpoint"
    );
    handles.server_task.abort();
    handles.worker_task.abort();
}

#[tokio::test]
async fn js_fetches_and_applies_i18n_catalog() {
    let (handles, _tmp) = start_daemon().await;
    let js = fetch_bundle_js(handles.local_addr).await;
    // Identifier names (`initI18n`) get mangled by the minifier so
    // the grep targets unminifiable strings: the API URL and the
    // DOM attribute string the catalog applies to.
    assert!(js.contains("/i18n.json"), "bundle should fetch /i18n.json");
    assert!(
        js.contains("data-i18n"),
        "bundle should walk [data-i18n] elements",
    );
    handles.server_task.abort();
    handles.worker_task.abort();
}

#[tokio::test]
async fn new_job_panel_present_in_ssr_html() {
    let (handles, _tmp) = start_daemon().await;
    let url = format!("http://{}/", handles.local_addr);
    let body = reqwest::get(&url).await.unwrap().text().await.unwrap();
    // The New Job button + form should ship pre-rendered so the
    // operator sees the trigger affordance before any JS runs.
    assert!(
        body.contains(r#"id="open-new-job""#),
        "New Job button missing from SSR HTML",
    );
    assert!(
        body.contains(r#"id="new-job-form""#),
        "New Job form missing from SSR HTML",
    );
    // Variant fieldsets for every supported JobKind.
    for kind in [
        "fuzz",
        "boot-river-elf",
        "load-aegis-bitstream",
        "run-aegis-vector",
        "named",
    ] {
        assert!(
            body.contains(&format!(r#"nj-kind nj-kind-{kind} hidden"#)),
            "missing fieldset for kind `{kind}`",
        );
    }
    handles.server_task.abort();
    handles.worker_task.abort();
}

#[tokio::test]
async fn bundle_wires_new_job_submit_and_cancel_to_jobs_api() {
    let (handles, _tmp) = start_daemon().await;
    let js = fetch_bundle_js(handles.local_addr).await;
    // Submit handler POSTs to /jobs; cancel handler POSTs to
    // /jobs/<id>/cancel. Both URL fragments are string literals
    // minification preserves verbatim.
    assert!(
        js.contains("/jobs/") && js.contains("/cancel"),
        "bundle should reference /jobs/<id>/cancel",
    );
    assert!(
        js.contains("data-cancel-job"),
        "bundle should emit a [data-cancel-job] attribute on the cancel button",
    );
    assert!(
        js.contains("/restart") && js.contains("data-restart-job"),
        "bundle should reference /jobs/<id>/restart and emit a \
         [data-restart-job] button attribute",
    );
    // FileReader/base64 helper signal: the kebab-case JobKind
    // discriminators land in the wire payload as quoted strings.
    for k in [
        "mock-hello",
        "fuzz",
        "boot-river-elf",
        "load-aegis-bitstream",
        "run-aegis-vector",
    ] {
        assert!(js.contains(k), "bundle should mention JobKind tag `{k}`",);
    }
    handles.server_task.abort();
    handles.worker_task.abort();
}

#[tokio::test]
async fn js_renders_connection_status_badge() {
    let (handles, _tmp) = start_daemon().await;
    let js = fetch_bundle_js(handles.local_addr).await;
    let css_url = format!("http://{}/assets/app.css", handles.local_addr);
    let css = reqwest::get(&css_url).await.unwrap().text().await.unwrap();
    // JS must read the field name and render a status pill class.
    // Both are string literals minification preserves.
    assert!(
        js.contains("connection_status"),
        "bundle should read d.connection_status",
    );
    assert!(
        js.contains("dut-status-"),
        "bundle should emit a dut-status-<state> class",
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
