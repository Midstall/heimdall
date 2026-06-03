use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use heimdall_core::DutId;
use serde::{Deserialize, Serialize};

use crate::campaign::{refresh_state, submit_campaign};
use crate::server::AppState;
use crate::types::{Campaign, CampaignId, CampaignTemplate, Job};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/campaigns", get(list).post(create))
        .route("/campaigns/:id", get(get_one))
        .route("/campaigns/:id/report.json", get(report_json))
}

#[derive(Debug, Deserialize)]
pub struct CreateCampaign {
    pub dut: DutId,
    pub template: CampaignTemplate,
    #[serde(default)]
    pub chip_serial: Option<String>,
}

#[derive(Serialize)]
struct ListResponse {
    campaigns: Vec<Campaign>,
}

async fn list(State(app): State<AppState>) -> Result<Json<ListResponse>, ApiError> {
    let campaigns = app
        .store
        .list_campaigns(None)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(ListResponse { campaigns }))
}

async fn create(
    State(app): State<AppState>,
    Json(body): Json<CreateCampaign>,
) -> Result<(StatusCode, Json<Campaign>), ApiError> {
    let dut_record = app
        .dut_registry
        .lookup(&body.dut)
        .ok_or_else(|| ApiError::BadRequest(format!("unknown dut `{}`", body.dut.0)))?;
    let dut_kind = dut_record.kind;
    let bringup = dut_record.bringup.as_ref();
    let campaign = submit_campaign(
        &app.queue,
        body.template,
        body.dut,
        dut_kind,
        body.chip_serial,
        bringup,
    )
    .await
    .map_err(ApiError::from)?;
    Ok((StatusCode::CREATED, Json(campaign)))
}

async fn get_one(
    State(app): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<Campaign>, ApiError> {
    let id = CampaignId(id);
    let _ = refresh_state(&app.queue, id)
        .await
        .map_err(ApiError::from)?;
    let campaign = app
        .store
        .get_campaign(id)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(campaign))
}

#[derive(Serialize)]
struct CampaignReport {
    campaign: Campaign,
    jobs: Vec<Job>,
}

async fn report_json(
    State(app): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<CampaignReport>, ApiError> {
    let id = CampaignId(id);
    let _ = refresh_state(&app.queue, id)
        .await
        .map_err(ApiError::from)?;
    let campaign = app
        .store
        .get_campaign(id)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;
    let jobs = app
        .store
        .list_jobs_for_campaign(id)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(CampaignReport { campaign, jobs }))
}

// Reuse ApiError from the jobs route module.
use crate::routes::jobs::ApiError;
