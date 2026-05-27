//! GET /metrics: Prometheus text-format scrape endpoint.
//!
//! Emits gauges derived from the live JobStore + DutRegistry + LeaseManager
//! plus a process uptime gauge. No external dependency, the text format is
//! trivial enough to render by hand.

use std::fmt::Write as _;
use std::time::Instant;

use axum::{Router, extract::State, http::header, response::IntoResponse, routing::get};

use crate::server::AppState;
use crate::types::{JobFilter, JobState, VerdictSummary};

pub fn router() -> Router<AppState> {
    Router::new().route("/metrics", get(metrics))
}

async fn metrics(State(app): State<AppState>) -> impl IntoResponse {
    let started = app.started_at;
    let uptime = started.elapsed().as_secs_f64();

    // Pull a snapshot of everything we expose.
    let jobs = app
        .store
        .list_jobs(JobFilter::default())
        .await
        .unwrap_or_default();
    let leases = app.leases.list().await;
    let dut_count = app.dut_registry.iter().count();

    // Bucket jobs.
    let mut queued = 0u64;
    let mut running = 0u64;
    let mut done = 0u64;
    let mut failed = 0u64;
    let mut cancelled = 0u64;
    let mut verdict_pass = 0u64;
    let mut verdict_fail = 0u64;
    let mut verdict_skip = 0u64;
    let mut verdict_error = 0u64;
    for job in &jobs {
        match &job.state {
            JobState::Queued => queued += 1,
            JobState::Running => running += 1,
            JobState::Done(v) => {
                done += 1;
                match v {
                    VerdictSummary::Pass => verdict_pass += 1,
                    VerdictSummary::Fail { .. } => verdict_fail += 1,
                    VerdictSummary::Skip { .. } => verdict_skip += 1,
                    VerdictSummary::Error { .. } => verdict_error += 1,
                }
            }
            JobState::Failed(_) => failed += 1,
            JobState::Cancelled => cancelled += 1,
        }
    }

    let mut body = String::with_capacity(2048);
    let version = heimdall_core::VERSION;

    let _ = writeln!(
        body,
        "# HELP heimdall_build_info Build info as labels; value is always 1.\n\
         # TYPE heimdall_build_info gauge\n\
         heimdall_build_info{{version=\"{version}\"}} 1"
    );
    let _ = writeln!(
        body,
        "# HELP heimdall_uptime_seconds Seconds since the daemon started.\n\
         # TYPE heimdall_uptime_seconds gauge\n\
         heimdall_uptime_seconds {uptime}"
    );
    let _ = writeln!(
        body,
        "# HELP heimdall_jobs Number of jobs in each lifecycle state.\n\
         # TYPE heimdall_jobs gauge\n\
         heimdall_jobs{{state=\"queued\"}} {queued}\n\
         heimdall_jobs{{state=\"running\"}} {running}\n\
         heimdall_jobs{{state=\"done\"}} {done}\n\
         heimdall_jobs{{state=\"failed\"}} {failed}\n\
         heimdall_jobs{{state=\"cancelled\"}} {cancelled}"
    );
    let _ = writeln!(
        body,
        "# HELP heimdall_verdicts Done jobs broken down by verdict.\n\
         # TYPE heimdall_verdicts gauge\n\
         heimdall_verdicts{{result=\"pass\"}} {verdict_pass}\n\
         heimdall_verdicts{{result=\"fail\"}} {verdict_fail}\n\
         heimdall_verdicts{{result=\"skip\"}} {verdict_skip}\n\
         heimdall_verdicts{{result=\"error\"}} {verdict_error}"
    );
    let _ = writeln!(
        body,
        "# HELP heimdall_duts_configured Number of DUTs registered in heimdall.toml.\n\
         # TYPE heimdall_duts_configured gauge\n\
         heimdall_duts_configured {dut_count}"
    );
    let _ = writeln!(
        body,
        "# HELP heimdall_leases_active Number of currently-held DUT leases.\n\
         # TYPE heimdall_leases_active gauge\n\
         heimdall_leases_active {}",
        leases.len()
    );

    (
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
}

/// Tracking instant used to compute the uptime gauge. Lives on AppState so a
/// single `started_at` is shared across the whole process.
pub fn started_at() -> Instant {
    Instant::now()
}
