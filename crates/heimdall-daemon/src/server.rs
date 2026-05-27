//! Axum server: builds the app from AppState. Routes live under src/routes/.

use std::sync::Arc;
use std::time::Instant;

use axum::Router;
use tower_http::trace::TraceLayer;

use crate::dut_registry::DutRegistry;
use crate::event_bus::EventBus;
use crate::lease::LeaseManager;
use crate::queue::JobQueue;
use crate::store::{BlobStore, JobStore};

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<dyn JobStore>,
    pub blobs: Arc<dyn BlobStore>,
    pub bus: EventBus,
    pub leases: LeaseManager,
    pub queue: JobQueue,
    pub dut_registry: Arc<DutRegistry>,
    /// Instant the daemon process started. Used by `/metrics` for uptime.
    pub started_at: Instant,
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .merge(crate::routes::health::router())
        .merge(crate::routes::jobs::router())
        .merge(crate::routes::duts::router())
        .merge(crate::routes::campaigns::router())
        .merge(crate::routes::events::router())
        .merge(crate::routes::metrics::router())
        .merge(crate::routes::i18n::router())
        .merge(crate::routes::web::router())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
