//! Heimdall daemon: rig-side mode. Trusted environment only; no auth.
//!
//! Provides JobStore + BlobStore traits with sqlite + local-fs default
//! impls, an axum HTTP + WS server, and a worker that runs queued jobs
//! through `heimdall_test::Runner`.

pub mod campaign;
pub mod dump;
pub mod dut_registry;
pub mod error;
pub mod event_bus;
pub mod factory;
pub mod lease;
pub mod queue;
pub mod routes;
pub mod runtime;
pub mod server;
pub mod store;
pub mod templates;
pub mod types;
pub mod worker;

pub use campaign::{compute_state, refresh_state, submit_campaign};
pub use dut_registry::{
    BringupPayload, ConnectionStatus, DutRecord, DutRegistry, GoldenSpec, GpioSpec, IoPinmap,
    PadDirection, PadEntry, TransportSpec, build_registry, build_registry_with_root,
};
pub use error::{DaemonError, Result};
pub use event_bus::EventBus;
#[cfg(feature = "aegis")]
pub use factory::{AegisLoadMockFactory, AegisRealFactory, AegisVectorTest};
#[cfg(feature = "river")]
pub use factory::{BootRiverElfTest, RiverRealFactory};
pub use factory::{DispatchBundle, DriverFactory, DriverRegistry, MockHelloFactory};
pub use lease::{LeaseManager, LeaseTtl};
pub use queue::{JobQueue, JobQueueReceiver};
pub use runtime::{DaemonHandles, start, start_with_config, start_with_registry};
pub use server::{AppState, build_router};
pub use store::{BlobStore, JobStore, LocalFsBlobStore};
pub use worker::Worker;

#[cfg(feature = "sqlite")]
pub use store::SqliteJobStore;
pub use types::{
    Blob, BlobId, Campaign, CampaignId, CampaignState, CampaignTemplate, Event, EventId, Job,
    JobFilter, JobId, JobKind, JobState, JobStateTag, Lease, LeaseId, NewJob, VerdictSummary,
};
