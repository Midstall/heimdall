//! Daemon-side adapter that mounts the `heimdall-web` router and
//! supplies the SSR index data by walking the daemon's stores.
//!
//! The router itself, the templates, and the static-asset embed all
//! live in the `heimdall-web` crate. This module exists only to plug
//! the daemon's `AppState` into the [`heimdall_web::WebContext`]
//! trait the router consumes.

use async_trait::async_trait;
use axum::Router;
use heimdall_i18n::Locale;
use heimdall_web::{CampaignRow, DutCardRow, IndexData, JobRow, WebContext, WebContextError};

use crate::dut_registry::ConnectionStatus;
use crate::server::AppState;
use crate::types::{JobFilter, JobState, VerdictSummary};

pub fn router() -> Router<AppState> {
    heimdall_web::router::<AppState>()
}

#[async_trait]
impl WebContext for AppState {
    async fn build_index_data(&self, locale: Locale) -> Result<IndexData, WebContextError> {
        let jobs_raw = self
            .store
            .list_jobs(JobFilter::default())
            .await
            .map_err(WebContextError::backend)?;
        let campaigns_raw = self
            .store
            .list_campaigns(Some(50))
            .await
            .map_err(WebContextError::backend)?;
        let duts_raw: Vec<_> = self.dut_registry.iter().cloned().collect();
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

        Ok(IndexData {
            jobs,
            campaigns,
            duts,
        })
    }
}

fn short_uuid(s: &str) -> String {
    s.chars().take(8).collect()
}

fn job_kind_str(kind: &crate::types::JobKind) -> String {
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
        JobState::Dead => "state-dead".into(),
    }
}

fn job_state_label(state: &JobState) -> String {
    match state {
        JobState::Queued => "queued".into(),
        JobState::Running => "running".into(),
        JobState::Done(v) => format!("done/{}", verdict_kind(v)),
        JobState::Failed(msg) => format!("failed: {msg}"),
        JobState::Cancelled => "cancelled".into(),
        JobState::Dead => "dead".into(),
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
