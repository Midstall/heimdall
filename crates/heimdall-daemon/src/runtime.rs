//! Daemon runtime: ties together store, blob store, event bus, lease
//! manager, queue, worker, and HTTP server.

use std::net::SocketAddr;
use std::sync::Arc;

use heimdall_config::ConfigFile;
use tokio::net::TcpListener;
use tracing::warn;

use crate::dut_registry::{DutRegistry, build_registry};
use crate::error::{DaemonError, Result};
use crate::event_bus::EventBus;
use crate::factory::DriverRegistry;
use crate::lease::{LeaseManager, LeaseTtl};
use crate::queue::JobQueue;
use crate::server::{AppState, build_router};
use crate::store::{BlobStore, JobStore};
use crate::worker::Worker;

pub struct DaemonHandles {
    pub local_addr: SocketAddr,
    pub server_task: tokio::task::JoinHandle<()>,
    pub worker_task: tokio::task::JoinHandle<()>,
}

pub async fn start(
    bind: SocketAddr,
    store: Arc<dyn JobStore>,
    blobs: Arc<dyn BlobStore>,
) -> Result<DaemonHandles> {
    start_inner(
        bind,
        store,
        blobs,
        Arc::new(DutRegistry::new()),
        DriverRegistry::default_mock(),
    )
    .await
}

pub async fn start_with_registry(
    bind: SocketAddr,
    store: Arc<dyn JobStore>,
    blobs: Arc<dyn BlobStore>,
    registry: DriverRegistry,
) -> Result<DaemonHandles> {
    start_inner(bind, store, blobs, Arc::new(DutRegistry::new()), registry).await
}

/// Build a DutRegistry from the config file and start the daemon with both
/// the dut registry and a driver registry that includes AegisRealFactory
/// (when the `aegis` feature is enabled).
pub async fn start_with_config(
    bind: SocketAddr,
    store: Arc<dyn JobStore>,
    blobs: Arc<dyn BlobStore>,
    config: &ConfigFile,
) -> Result<DaemonHandles> {
    let dut_registry = Arc::new(build_registry(config)?);
    let driver_registry = DriverRegistry::default_with_registry(dut_registry.clone());
    start_inner(bind, store, blobs, dut_registry, driver_registry).await
}

async fn start_inner(
    bind: SocketAddr,
    store: Arc<dyn JobStore>,
    blobs: Arc<dyn BlobStore>,
    dut_registry: Arc<DutRegistry>,
    driver_registry: DriverRegistry,
) -> Result<DaemonHandles> {
    if !bind.ip().is_loopback() {
        heimdall_i18n::lwarn!("log.daemon.non_loopback_bind", addr = bind);
    }

    let bus = EventBus::new(store.clone(), 1024);
    let leases = LeaseManager::new(LeaseTtl::default());
    let (queue, recv) = JobQueue::new(store.clone(), bus.clone());

    let state = AppState {
        store: store.clone(),
        blobs,
        bus: bus.clone(),
        leases: leases.clone(),
        queue: queue.clone(),
        dut_registry,
        started_at: std::time::Instant::now(),
    };

    let worker = Worker::new_with_registry(queue, leases, driver_registry);
    let worker_task = tokio::spawn(async move { worker.run(recv).await });

    let app = build_router(state);
    let listener = TcpListener::bind(bind).await.map_err(DaemonError::Io)?;
    let local_addr = listener.local_addr().map_err(DaemonError::Io)?;
    heimdall_i18n::linfo!("log.daemon.listening", addr = local_addr);
    let server_task = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            warn!(error = %e, "axum serve exited");
        }
    });

    Ok(DaemonHandles {
        local_addr,
        server_task,
        worker_task,
    })
}
