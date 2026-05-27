use axum::{Json, Router, routing::get};
use serde::Serialize;

use crate::server::AppState;

#[derive(Serialize)]
struct Health {
    status: &'static str,
    version: &'static str,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/health", get(health))
}

async fn health() -> Json<Health> {
    Json(Health {
        status: "ok",
        version: heimdall_core::VERSION,
    })
}
