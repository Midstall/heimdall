//! Web UI: SSR for the initial `/` render (so first paint already shows
//! current jobs/campaigns/DUTs with the user's locale applied) plus
//! /assets/<path> for the CSS+JS bundle. The JS layer continues to CSR
//! live updates via /events + tick polling.

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use rust_embed::RustEmbed;
use serde::Deserialize;

use askama::Template;
use heimdall_i18n::Locale;

use crate::dut_registry::ConnectionStatus;
use crate::server::AppState;
use crate::types::{JobFilter, JobState, VerdictSummary};

// Assets dir is assembled by build.rs into $OUT_DIR/assets: static files
// (app.css, app.js) copied from `assets/`, plus SVGs rendered by the
// heimdall-logo Python package or pulled from $HEIMDALL_LOGO_SVGS.
#[derive(RustEmbed)]
#[folder = "$OUT_DIR/assets/"]
struct Assets;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(index))
        .route("/assets/*path", get(asset))
}

#[derive(Deserialize)]
struct LangQuery {
    lang: Option<String>,
}

async fn index(
    State(app): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<LangQuery>,
) -> Response {
    let locale = resolve_locale(&q.lang, &headers);
    let ctx = match build_index_ctx(app, locale).await {
        Ok(c) => c,
        Err(e) => {
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from(format!("index render failed: {e}")))
                .unwrap();
        }
    };
    match ctx.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Body::from(format!("template error: {e}")))
            .unwrap(),
    }
}

async fn asset(Path(path): Path<String>) -> Response {
    match Assets::get(&path) {
        Some(file) => {
            let mime = mime_guess::from_path(&path).first_or_octet_stream();
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime.as_ref())
                .body(Body::from(file.data.into_owned()))
                .unwrap()
        }
        None => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("not found"))
            .unwrap(),
    }
}

fn resolve_locale(query: &Option<String>, headers: &HeaderMap) -> Locale {
    if let Some(s) = query.as_deref() {
        if let Some(l) = Locale::from_tag(s) {
            return l;
        }
    }
    if let Some(val) = headers
        .get(header::ACCEPT_LANGUAGE)
        .and_then(|v| v.to_str().ok())
    {
        for chunk in val.split(',') {
            let tag = chunk.split(';').next().unwrap_or("").trim();
            if let Some(l) = Locale::from_tag(tag) {
                return l;
            }
        }
    }
    Locale::En
}

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate {
    locale: String,
    jobs: Vec<JobRow>,
    campaigns: Vec<CampaignRow>,
    duts: Vec<DutCardRow>,
    /// Locale used for `self.trans()` lookups. Held as enum so we don't have
    /// to round-trip through a string per call.
    #[allow(dead_code)]
    locale_kind: Locale,
}

impl IndexTemplate {
    /// Translation helper called from the template.
    pub fn trans(&self, key: &str) -> String {
        heimdall_i18n::t_in(self.locale_kind, key)
    }
}

struct JobRow {
    id_short: String,
    dut: String,
    kind: String,
    state_class: String,
    state_label: String,
    created_at: String,
}

struct CampaignRow {
    id_short: String,
    dut: String,
    template: String,
    state: String,
    chip_serial: String,
}

struct DutCardRow {
    id: String,
    kind: String,
    chip_serial: String,
    jtag_driver: String,
    status_class: &'static str,
    status_label: String,
    has_netlist: bool,
}

async fn build_index_ctx(
    app: AppState,
    locale: Locale,
) -> Result<IndexTemplate, crate::error::DaemonError> {
    let jobs_raw = app.store.list_jobs(JobFilter::default()).await?;
    let campaigns_raw = app.store.list_campaigns(Some(50)).await?;
    let duts_raw: Vec<_> = app.dut_registry.iter().cloned().collect();
    let probes = duts_raw.iter().map(|d| d.jtag.probe_connection());
    let statuses: Vec<_> = futures::future::join_all(probes).await;

    let jobs: Vec<JobRow> = jobs_raw
        .into_iter()
        .map(|j| JobRow {
            id_short: short_uuid(&j.id.0.to_string()),
            dut: j.dut.0.clone(),
            kind: job_kind_str(&j.kind),
            state_class: job_state_class(&j.state),
            state_label: job_state_label(&j.state),
            created_at: j.created_at.format("%Y-%m-%d %H:%M:%S").to_string(),
        })
        .collect();

    let campaigns: Vec<CampaignRow> = campaigns_raw
        .into_iter()
        .map(|c| CampaignRow {
            id_short: short_uuid(&c.id.0.to_string()),
            dut: c.dut.0.clone(),
            template: c.template.name().to_string(),
            state: format!("{:?}", c.state).to_lowercase(),
            chip_serial: c.chip_serial.clone().unwrap_or_else(|| "-".into()),
        })
        .collect();

    let duts: Vec<DutCardRow> = duts_raw
        .into_iter()
        .zip(statuses)
        .map(|(d, status)| {
            let status_class = match status {
                ConnectionStatus::Connected => "connected",
                ConnectionStatus::Disconnected => "disconnected",
                ConnectionStatus::Unknown => "idle",
            };
            let status_label_key = match status {
                ConnectionStatus::Connected => "common.status.connected",
                ConnectionStatus::Disconnected => "common.status.disconnected",
                ConnectionStatus::Unknown => "common.status.idle",
            };
            DutCardRow {
                id: d.id.0.clone(),
                kind: dut_kind_str(d.kind),
                chip_serial: d.chip_serial.clone().unwrap_or_else(|| "-".into()),
                jtag_driver: jtag_driver_name(&d.jtag).to_string(),
                status_class,
                status_label: heimdall_i18n::t_in(locale, status_label_key),
                has_netlist: d.netlist.is_some(),
            }
        })
        .collect();

    Ok(IndexTemplate {
        locale: locale.code().to_string(),
        jobs,
        campaigns,
        duts,
        locale_kind: locale,
    })
}

fn short_uuid(s: &str) -> String {
    s.chars().take(8).collect()
}

fn job_kind_str(kind: &crate::types::JobKind) -> String {
    // JobKind uses `#[serde(tag = "kind")]` with kebab-case variant names;
    // extract that discriminator so a kind added later still renders without
    // forcing a recompile here.
    serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.get("kind").and_then(|k| k.as_str()).map(str::to_owned))
        .unwrap_or_else(|| "unknown".into())
}

fn job_state_class(state: &JobState) -> String {
    match state {
        JobState::Queued => "state-queued".into(),
        JobState::Running => "state-running".into(),
        JobState::Done(v) => format!("state-done verdict-{}", verdict_kind(v)),
        JobState::Failed(_) => "state-failed".into(),
        JobState::Cancelled => "state-cancelled".into(),
    }
}

fn job_state_label(state: &JobState) -> String {
    match state {
        JobState::Queued => "queued".into(),
        JobState::Running => "running".into(),
        JobState::Done(v) => format!("done/{}", verdict_kind(v)),
        JobState::Failed(msg) => format!("failed: {msg}"),
        JobState::Cancelled => "cancelled".into(),
    }
}

fn verdict_kind(v: &VerdictSummary) -> &'static str {
    match v {
        VerdictSummary::Pass => "pass",
        VerdictSummary::Fail { .. } => "fail",
        VerdictSummary::Skip { .. } => "skip",
        VerdictSummary::Error { .. } => "error",
    }
}

fn dut_kind_str(kind: heimdall_core::DutKind) -> String {
    // DutKind serializes as a kebab-case JSON string. Trim quotes.
    serde_json::to_string(&kind)
        .unwrap_or_else(|_| "\"custom\"".into())
        .trim_matches('"')
        .to_string()
}

fn jtag_driver_name(spec: &crate::dut_registry::TransportSpec) -> &'static str {
    use crate::dut_registry::TransportSpec;
    match spec {
        TransportSpec::Mock => "mock",
        TransportSpec::BitbangCdev { .. } => "bitbang-jtag",
        TransportSpec::Openocd { .. } => "openocd",
        TransportSpec::OpenocdSpawned { .. } => "openocd-spawn",
        TransportSpec::Ftdi { .. } => "ftdi",
    }
}
