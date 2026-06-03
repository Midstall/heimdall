use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

use crate::server::AppState;
use crate::types::{EventId, EventRecord, Job, JobFilter, JobId, JobState, JobStateTag, NewJob};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/jobs", get(list).post(create).delete(prune))
        .route("/jobs/:id", get(get_one).delete(delete_one))
        .route("/jobs/:id/cancel", post(cancel))
        .route("/jobs/:id/restart", post(restart))
        .route("/jobs/:id/logs", get(logs))
}

#[derive(Deserialize)]
struct ListQuery {
    dut: Option<String>,
    state: Option<JobStateTag>,
    limit: Option<u32>,
    /// Row offset for pagination. Combined with `limit`, the UI can
    /// walk pages without holding a server-side cursor. `0` =
    /// first page, which is also the response when omitted.
    offset: Option<u32>,
}

#[derive(Serialize)]
struct ListResponse {
    jobs: Vec<Job>,
    /// Total rows matching `filter` ignoring `limit`/`offset`. The
    /// UI reads this to render "page X of N" without firing a
    /// second request for the count.
    total: u64,
}

async fn list(
    State(app): State<AppState>,
    Query(q): Query<ListQuery>,
) -> Result<Json<ListResponse>, ApiError> {
    let base = JobFilter {
        dut: q.dut.clone().map(heimdall_core::DutId),
        state_in: q.state.map(|s| vec![s]),
        ..JobFilter::default()
    };
    // Total is the count of the unpaginated query; the listing
    // returns the actual page.
    let total = app
        .store
        .count_jobs(base.clone())
        .await
        .map_err(ApiError::from)?;
    let paged = JobFilter {
        limit: q.limit,
        offset: q.offset,
        ..base
    };
    let jobs = app.store.list_jobs(paged).await.map_err(ApiError::from)?;
    Ok(Json(ListResponse { jobs, total }))
}

async fn create(
    State(app): State<AppState>,
    Json(new): Json<NewJob>,
) -> Result<(StatusCode, Json<Job>), ApiError> {
    let dut_kind = app
        .dut_registry
        .lookup(&new.dut)
        .map(|rec| rec.kind)
        .ok_or_else(|| ApiError::BadRequest(format!("unknown dut `{}`", new.dut.0)))?;
    let job = app
        .queue
        .submit(new, dut_kind)
        .await
        .map_err(ApiError::from)?;
    Ok((StatusCode::CREATED, Json(job)))
}

async fn get_one(
    State(app): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<Job>, ApiError> {
    let job = app
        .store
        .get_job(JobId(id))
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(job))
}

#[derive(Serialize)]
struct JobLogsResponse {
    logs: Vec<EventRecord>,
}

#[derive(Deserialize)]
struct LogsQuery {
    #[serde(default)]
    since: Option<u64>,
    #[serde(default)]
    limit: Option<u32>,
}

const JOB_LOGS_DEFAULT_LIMIT: u32 = 500;
const JOB_LOGS_MAX_LIMIT: u32 = 5000;

async fn logs(
    State(app): State<AppState>,
    Path(id): Path<uuid::Uuid>,
    Query(q): Query<LogsQuery>,
) -> Result<Json<JobLogsResponse>, ApiError> {
    let since = EventId(q.since.unwrap_or(0));
    let limit = q
        .limit
        .unwrap_or(JOB_LOGS_DEFAULT_LIMIT)
        .min(JOB_LOGS_MAX_LIMIT);
    let logs = app
        .store
        .list_job_logs(JobId(id), since, limit)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(JobLogsResponse { logs }))
}

async fn cancel(
    State(app): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> Result<StatusCode, ApiError> {
    let job_id = JobId(id);
    let job = app
        .store
        .get_job(job_id)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;
    match job.state {
        JobState::Queued => {
            app.queue
                .transition(job_id, JobState::Cancelled)
                .await
                .map_err(ApiError::from)?;
            Ok(StatusCode::NO_CONTENT)
        }
        JobState::Running => {
            // Fire the in-flight token; the worker is racing its
            // run future against this token and will land on
            // `JobState::Cancelled` once it observes the signal.
            // If nothing was registered (e.g. job just transitioned
            // to terminal before we got here), surface a clear error
            // instead of silently flipping state out from under the
            // worker.
            if !app.cancellations.cancel(job_id) {
                return Err(ApiError::BadRequest(
                    "running job has no cancellation handle registered".to_string(),
                ));
            }
            Ok(StatusCode::NO_CONTENT)
        }
        other => Err(ApiError::BadRequest(format!(
            "cannot cancel job in state {:?}",
            other
        ))),
    }
}

/// Re-submit a finished job as a fresh entry. The old row is left
/// untouched (history matters: a `Dead` audit trail vanishing on
/// restart would hide infrastructure incidents). The response
/// returns the *new* job, freshly created, freshly queued.
///
/// Only terminal states qualify: `Done`, `Failed`, `Cancelled`,
/// `Dead`. Restarting a `Queued` or `Running` job is rejected so
/// operators can't accidentally double-book a DUT mid-flight.
async fn restart(
    State(app): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> Result<(StatusCode, Json<Job>), ApiError> {
    let job_id = JobId(id);
    let old = app
        .store
        .get_job(job_id)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;
    match old.state.tag() {
        JobStateTag::Queued | JobStateTag::Running => {
            return Err(ApiError::BadRequest(format!(
                "cannot restart job in non-terminal state {:?}",
                old.state
            )));
        }
        JobStateTag::Done | JobStateTag::Failed | JobStateTag::Cancelled | JobStateTag::Dead => {}
    }
    // The DUT may have been removed from heimdall.toml since the
    // original run. Surface that as a 400 rather than panicking or
    // creating an unresolvable job.
    let dut_kind = app
        .dut_registry
        .lookup(&old.dut)
        .map(|rec| rec.kind)
        .ok_or_else(|| {
            ApiError::BadRequest(format!(
                "dut `{}` is no longer registered; cannot restart",
                old.dut.0
            ))
        })?;
    let new = NewJob {
        dut: old.dut.clone(),
        kind: old.kind.clone(),
        campaign: old.campaign,
    };
    let job = app
        .queue
        .submit(new, dut_kind)
        .await
        .map_err(ApiError::from)?;
    Ok((StatusCode::CREATED, Json(job)))
}

/// DELETE /jobs/:id drops a single terminal job and its associated
/// rows. Refuses non-terminal jobs (the store maps that to a
/// Config error which we surface as 400). Returns 204 on success,
/// 404 when the job doesn't exist.
async fn delete_one(
    State(app): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> Result<StatusCode, ApiError> {
    let job_id = JobId(id);
    match app.store.delete_job(job_id).await {
        Ok(true) => Ok(StatusCode::NO_CONTENT),
        Ok(false) => Err(ApiError::NotFound),
        // The store returns Config("refusing to delete non-terminal
        // job ...") when state isn't Done/Failed/Cancelled/Dead. Map
        // that to 400 so the UI can render a sane message instead of
        // an opaque 500.
        Err(crate::error::DaemonError::Config(msg)) => Err(ApiError::BadRequest(msg)),
        Err(e) => Err(ApiError::from(e)),
    }
}

#[derive(Deserialize)]
struct PruneQuery {
    /// Only delete jobs created strictly before this UTC instant
    /// (RFC 3339 / ISO 8601). When omitted, the cutoff is "now",
    /// which together with the terminal-only restriction means
    /// every Done / Failed / Cancelled / Dead row gets deleted.
    /// Operators should always supply this for routine cleanup.
    before: Option<chrono::DateTime<chrono::Utc>>,
    /// Restrict to a specific terminal state (Done / Failed /
    /// Cancelled / Dead). When omitted, all four are eligible.
    /// Non-terminal tags are silently filtered out by the store.
    state: Option<JobStateTag>,
}

#[derive(Serialize)]
struct PruneResponse {
    deleted: u64,
}

/// DELETE /jobs?before=...&state=... bulk-deletes every terminal
/// job matching the filter. Returns the count of rows actually
/// deleted. In-flight jobs are silently skipped (the store
/// enforces the terminal-only invariant).
async fn prune(
    State(app): State<AppState>,
    Query(q): Query<PruneQuery>,
) -> Result<Json<PruneResponse>, ApiError> {
    let filter = JobFilter {
        state_in: q.state.map(|s| vec![s]),
        created_before: q.before,
        ..JobFilter::default()
    };
    let deleted = app
        .store
        .delete_jobs(filter)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(PruneResponse { deleted }))
}

#[derive(Debug)]
pub enum ApiError {
    NotFound,
    BadRequest(String),
    Internal(String),
}

impl From<crate::error::DaemonError> for ApiError {
    fn from(e: crate::error::DaemonError) -> Self {
        ApiError::Internal(e.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let (status, msg) = match self {
            ApiError::NotFound => (StatusCode::NOT_FOUND, "not found".to_string()),
            ApiError::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            ApiError::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, m),
        };
        (status, Json(serde_json::json!({"error": msg}))).into_response()
    }
}
