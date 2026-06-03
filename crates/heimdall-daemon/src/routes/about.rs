//! `GET /about` returns the running daemon's version + feature
//! flags. Powers the web "About" tab and the TUI's about view.

use axum::{Json, Router, routing::get};

use crate::server::AppState;
use crate::types::{AboutInfo, current_about_info};

pub fn router() -> Router<AppState> {
    Router::new().route("/about", get(about))
}

async fn about() -> Json<AboutInfo> {
    Json(current_about_info())
}
