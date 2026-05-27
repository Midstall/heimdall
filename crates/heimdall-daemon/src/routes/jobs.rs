use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

use crate::server::AppState;
use crate::types::{Job, JobFilter, JobId, JobState, JobStateTag, NewJob};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/jobs", get(list).post(create))
        .route("/jobs/:id", get(get_one))
        .route("/jobs/:id/cancel", post(cancel))
}

#[derive(Deserialize)]
struct ListQuery {
    dut: Option<String>,
    state: Option<JobStateTag>,
    limit: Option<u32>,
}

#[derive(Serialize)]
struct ListResponse {
    jobs: Vec<Job>,
}

async fn list(
    State(app): State<AppState>,
    Query(q): Query<ListQuery>,
) -> Result<Json<ListResponse>, ApiError> {
    let filter = JobFilter {
        dut: q.dut.map(heimdall_core::DutId),
        state_in: q.state.map(|s| vec![s]),
        limit: q.limit,
    };
    let jobs = app.store.list_jobs(filter).await.map_err(ApiError::from)?;
    Ok(Json(ListResponse { jobs }))
}

async fn create(
    State(app): State<AppState>,
    Json(new): Json<NewJob>,
) -> Result<(StatusCode, Json<Job>), ApiError> {
    let job = app.queue.submit(new).await.map_err(ApiError::from)?;
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
    if !matches!(job.state, JobState::Queued) {
        return Err(ApiError::BadRequest(format!(
            "cannot cancel job in state {:?}",
            job.state
        )));
    }
    app.queue
        .transition(job_id, JobState::Cancelled)
        .await
        .map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
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
