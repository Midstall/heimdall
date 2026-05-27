//! Worker: pulls JobIds from the queue receiver, dispatches each through the
//! DriverRegistry, runs the resulting (driver, golden, test) trio via the
//! existing Runner, and transitions state via JobQueue.

use heimdall_driver::Dut;
use heimdall_test::Runner;
use tracing::{error, info, instrument};

use crate::error::Result;
use crate::factory::DriverRegistry;
use crate::lease::LeaseManager;
use crate::queue::{JobQueue, JobQueueReceiver};
use crate::types::{Event, JobId, JobState, VerdictSummary};

pub struct Worker {
    queue: JobQueue,
    leases: LeaseManager,
    registry: DriverRegistry,
}

impl Worker {
    pub fn new(queue: JobQueue, leases: LeaseManager) -> Self {
        Self::new_with_registry(queue, leases, DriverRegistry::default_mock())
    }

    pub fn new_with_registry(
        queue: JobQueue,
        leases: LeaseManager,
        registry: DriverRegistry,
    ) -> Self {
        Self {
            queue,
            leases,
            registry,
        }
    }

    #[instrument(skip(self, recv))]
    pub async fn run(self, mut recv: JobQueueReceiver) {
        let Self {
            queue,
            leases,
            registry,
        } = self;
        while let Some(job_id) = recv.rx.recv().await {
            let q = queue.clone();
            let l = leases.clone();
            let r = registry.clone();
            if let Err(e) = handle_one(job_id, q, l, r).await {
                error!(error = %e, "job dispatch failed");
            }
        }
    }
}

#[instrument(skip(queue, leases, registry))]
async fn handle_one(
    job_id: JobId,
    queue: JobQueue,
    leases: LeaseManager,
    registry: DriverRegistry,
) -> Result<()> {
    let store = queue.store();
    let job = match store.get_job(job_id).await? {
        Some(j) => j,
        None => return Ok(()),
    };

    queue.transition(job_id, JobState::Running).await?;
    let lease = leases.acquire(job.dut.clone(), job_id).await?;
    queue
        .bus()
        .publish(Event::LeaseAcquired {
            lease: lease.id,
            dut: job.dut.clone(),
            holder: job_id,
        })
        .await?;

    let outcome = run_via_registry(&job, &registry).await;
    let next_state = match &outcome {
        Ok(verdict) => JobState::Done(VerdictSummary::from(verdict)),
        Err(msg) => JobState::Failed(msg.clone()),
    };

    queue.transition(job_id, next_state).await?;
    leases.release(&job.dut, lease.id).await?;
    queue
        .bus()
        .publish(Event::LeaseReleased {
            lease: lease.id,
            dut: job.dut.clone(),
        })
        .await?;
    info!(job = %job_id, "completed");
    Ok(())
}

async fn run_via_registry(
    job: &crate::types::Job,
    registry: &DriverRegistry,
) -> std::result::Result<heimdall_core::Verdict, String> {
    let bundle = registry.dispatch(job).await.map_err(|e| e.to_string())?;
    let runner = Runner::builder().build();
    let mut driver = bundle.driver;
    let mut golden = bundle.golden;
    let test = bundle.test;
    let mut dut = Dut::new(job.dut.clone(), job.dut_kind);
    let res = runner
        .run_one(&*test, &mut dut, &mut *driver, &mut *golden)
        .await
        .map_err(|e| e.to_string())?;
    Ok(res.verdict)
}

/// Convenience type alias matching the worker's JobStore expectation.
pub type SharedJobStore = std::sync::Arc<dyn crate::store::JobStore>;
