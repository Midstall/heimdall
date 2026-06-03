//! Axum server: builds the app from AppState. Routes live under src/routes/.

use std::sync::Arc;
use std::time::Instant;

use axum::Router;
use tower_http::trace::TraceLayer;

use crate::cancellations::CancellationRegistry;
use crate::dut_registry::DutRegistry;
use crate::dut_state::DutStateCache;
use crate::event_bus::EventBus;
use crate::isa_probes::IsaProbeCache;
use crate::lease::LeaseManager;
use crate::loaded_programs::LoadedProgramCache;
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
    pub dut_state_cache: DutStateCache,
    /// Per-job cache of "the program last loaded onto the DUT," used
    /// by the disasm route to render what a fuzz iter is executing.
    /// In-memory; daemon restart drops it and the next iter
    /// repopulates.
    pub loaded_programs: LoadedProgramCache,
    /// Per-DUT JTAG-probed ISA cache. Populated lazily by the fuzz
    /// worker the first time it dispatches against a given DUT; the
    /// result feeds the generator's enabled-extension set unless the
    /// operator overrode it via `[dut.isa]` in heimdall.toml.
    pub isa_probes: IsaProbeCache,
    /// In-flight job cancellation registry. The cancel HTTP route
    /// looks up by JobId and fires the token; the worker races its
    /// run future against the same token so the operator can stop
    /// running fuzz sessions from the web UI without waiting for
    /// the lease TTL.
    pub cancellations: CancellationRegistry,
    /// Instant the daemon process started. Used by `/metrics` for uptime.
    pub started_at: Instant,
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .merge(crate::routes::about::router())
        .merge(crate::routes::health::router())
        .merge(crate::routes::jobs::router())
        .merge(crate::routes::duts::router())
        .merge(crate::routes::campaigns::router())
        .merge(crate::routes::disasm::router())
        .merge(crate::routes::events::router())
        .merge(crate::routes::metrics::router())
        .merge(crate::routes::i18n::router())
        .merge(crate::routes::web::router())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
