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
use crate::types::{JobFilter, JobState, JobStateTag};
use crate::worker::Worker;

pub struct DaemonHandles {
    /// First bound address. Convenience alias for `local_addrs[0]`
    /// so single-bind callers (every existing test) can read it
    /// without indexing into a Vec.
    pub local_addr: SocketAddr,
    /// Every address the daemon is actually listening on, in the
    /// order they were bound. For single-bind callers this mirrors
    /// `local_addr` as a one-element vec.
    pub local_addrs: Vec<SocketAddr>,
    /// axum-serve task for the first bound listener. Single-bind
    /// callers abort this and `worker_task` to shut down cleanly.
    pub server_task: tokio::task::JoinHandle<()>,
    /// Any additional axum-serve tasks (one per bind beyond the
    /// first). Empty for single-bind starts. Multi-bind callers
    /// must abort both `server_task` and every entry here.
    pub extra_server_tasks: Vec<tokio::task::JoinHandle<()>>,
    pub worker_task: tokio::task::JoinHandle<()>,
}

impl DaemonHandles {
    /// Iterate over every server task: the first plus the extras.
    /// Caller can `.for_each(|t| t.abort())` to shut down every
    /// listener in one pass.
    pub fn server_tasks(&self) -> impl Iterator<Item = &tokio::task::JoinHandle<()>> {
        std::iter::once(&self.server_task).chain(self.extra_server_tasks.iter())
    }
}

pub async fn start(
    bind: SocketAddr,
    store: Arc<dyn JobStore>,
    blobs: Arc<dyn BlobStore>,
) -> Result<DaemonHandles> {
    start_inner(
        vec![bind],
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
    start_inner(
        vec![bind],
        store,
        blobs,
        Arc::new(DutRegistry::new()),
        registry,
    )
    .await
}

/// Start the daemon with a pre-populated DUT registry and the default mock
/// DriverRegistry. Useful for tests that need POST /jobs to find a matching
/// DUT without the overhead of loading a full ConfigFile.
pub async fn start_with_dut_registry(
    bind: SocketAddr,
    store: Arc<dyn JobStore>,
    blobs: Arc<dyn BlobStore>,
    dut_registry: Arc<DutRegistry>,
) -> Result<DaemonHandles> {
    start_inner(
        vec![bind],
        store,
        blobs,
        dut_registry,
        DriverRegistry::default_mock(),
    )
    .await
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
    start_inner(vec![bind], store, blobs, dut_registry, driver_registry).await
}

/// Multi-bind variant of [`start_with_config`]. Spawns one axum-serve
/// task per address in `binds`. Use this when the daemon needs to
/// answer on both IPv4-all and IPv6-all simultaneously, or any other
/// combination of explicit listeners. `binds` must be non-empty.
pub async fn start_with_config_binds(
    binds: Vec<SocketAddr>,
    store: Arc<dyn JobStore>,
    blobs: Arc<dyn BlobStore>,
    config: &ConfigFile,
) -> Result<DaemonHandles> {
    let dut_registry = Arc::new(build_registry(config)?);
    let driver_registry = DriverRegistry::default_with_registry(dut_registry.clone());
    start_inner(binds, store, blobs, dut_registry, driver_registry).await
}

/// Multi-bind variant of [`start`]. See [`start_with_config_binds`]
/// for the rationale. `binds` must be non-empty.
pub async fn start_binds(
    binds: Vec<SocketAddr>,
    store: Arc<dyn JobStore>,
    blobs: Arc<dyn BlobStore>,
) -> Result<DaemonHandles> {
    start_inner(
        binds,
        store,
        blobs,
        Arc::new(DutRegistry::new()),
        DriverRegistry::default_mock(),
    )
    .await
}

async fn start_inner(
    binds: Vec<SocketAddr>,
    store: Arc<dyn JobStore>,
    blobs: Arc<dyn BlobStore>,
    dut_registry: Arc<DutRegistry>,
    driver_registry: DriverRegistry,
) -> Result<DaemonHandles> {
    if binds.is_empty() {
        return Err(DaemonError::Config(
            "at least one bind address is required".into(),
        ));
    }
    for bind in &binds {
        if !bind.ip().is_loopback() {
            heimdall_i18n::lwarn!("log.daemon.non_loopback_bind", addr = bind);
        }
    }

    let bus = EventBus::new(store.clone(), 1024);
    let leases = LeaseManager::new(LeaseTtl::default());
    let (queue, recv) = JobQueue::new(store.clone(), bus.clone());

    // Reconcile any jobs the prior process left mid-flight. A row at
    // `Running` means the previous daemon was actively driving the
    // DUT when it died; we have no way to pick up where it left off
    // (hardware lease and in-memory state are gone), so flip to
    // `Dead`. The web UI renders these with a distinct class so
    // operators can spot them and re-submit.
    reconcile_orphaned_jobs(store.as_ref(), &queue).await?;
    let dut_state_cache = crate::dut_state::DutStateCache::new();
    // Background task that pipes DutStateSnapshot events into the cache.
    // Spawned before the worker so the very first run's snapshots land
    // in the cache. Aborted on shutdown alongside the other tasks via
    // the existing DaemonHandles teardown path.
    let _snapshot_task = crate::dut_state::spawn_snapshot_task(&bus, dut_state_cache.clone());

    let loaded_programs = crate::loaded_programs::LoadedProgramCache::new();
    let isa_probes = crate::isa_probes::IsaProbeCache::new();
    let cancellations = crate::cancellations::CancellationRegistry::new();
    let state = AppState {
        store: store.clone(),
        blobs,
        bus: bus.clone(),
        leases: leases.clone(),
        queue: queue.clone(),
        dut_registry,
        dut_state_cache,
        loaded_programs: loaded_programs.clone(),
        isa_probes: isa_probes.clone(),
        cancellations: cancellations.clone(),
        started_at: std::time::Instant::now(),
    };

    let worker = Worker::new_with_registry(queue, leases, driver_registry)
        .with_dut_registry(state.dut_registry.clone())
        .with_loaded_program_cache(loaded_programs)
        .with_isa_probe_cache(isa_probes)
        .with_blob_store(state.blobs.clone())
        .with_cancellation_registry(cancellations);
    let worker_task = tokio::spawn(async move { worker.run(recv).await });

    let app = build_router(state);
    let mut local_addrs = Vec::with_capacity(binds.len());
    let mut all_server_tasks = Vec::with_capacity(binds.len());
    for bind in &binds {
        let listener = TcpListener::bind(bind).await.map_err(DaemonError::Io)?;
        let local_addr = listener.local_addr().map_err(DaemonError::Io)?;
        heimdall_i18n::linfo!("log.daemon.listening", addr = local_addr);
        let app = app.clone();
        all_server_tasks.push(tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                warn!(error = %e, "axum serve exited");
            }
        }));
        local_addrs.push(local_addr);
    }
    let local_addr = local_addrs[0];
    let server_task = all_server_tasks.remove(0);
    let extra_server_tasks = all_server_tasks;

    Ok(DaemonHandles {
        local_addr,
        local_addrs,
        server_task,
        extra_server_tasks,
        worker_task,
    })
}

/// Walk the store for jobs the previous daemon process left at
/// `Running` and flip each to `JobState::Dead`. Called once at boot
/// before the worker starts pulling new jobs.
///
/// We deliberately don't try to resume Running jobs: their hardware
/// lease, cancellation token, and (for fuzz) per-iter program state
/// all lived in the prior process's memory and are now gone. Even
/// for tests that could in principle be replayed from scratch, the
/// DUT is in an unknown state. Operator re-submits, with a fresh
/// job id, is the only sound path.
///
/// Queued jobs are also stranded by a restart (the in-memory mpsc
/// channel is gone), but for a different reason: they never ran, so
/// the right move is to re-enqueue them so the new worker picks
/// them up. We push their ids back into the queue's send side.
async fn reconcile_orphaned_jobs(store: &dyn JobStore, queue: &JobQueue) -> Result<()> {
    let running = store
        .list_jobs(JobFilter {
            state_in: Some(vec![JobStateTag::Running]),
            ..JobFilter::default()
        })
        .await?;
    for job in &running {
        queue.transition(job.id, JobState::Dead).await?;
        heimdall_i18n::lwarn!("log.daemon.reconcile_dead", job = job.id.0);
    }

    let queued = store
        .list_jobs(JobFilter {
            state_in: Some(vec![JobStateTag::Queued]),
            ..JobFilter::default()
        })
        .await?;
    for job in &queued {
        // Best-effort: if the worker hasn't been spawned yet, the
        // receiver is still alive and `send` succeeds. If the worker
        // already exited (shouldn't happen at boot but guard anyway)
        // we surface a Config error rather than silently dropping
        // the work.
        queue.requeue(job.id)?;
        heimdall_i18n::linfo!("log.daemon.reconcile_requeue", job = job.id.0);
    }

    if !running.is_empty() || !queued.is_empty() {
        heimdall_i18n::linfo!(
            "log.daemon.reconcile_summary",
            dead = running.len(),
            requeued = queued.len()
        );
    }
    Ok(())
}
