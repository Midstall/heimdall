//! Combined JobStore + in-memory dispatch queue. The store provides
//! durability; the mpsc channel decouples job submission from worker
//! pickup.

use std::sync::Arc;

use tokio::sync::mpsc;
use tracing::debug;

use crate::error::Result;
use crate::event_bus::EventBus;
use crate::store::JobStore;
use crate::types::{Event, Job, JobId, JobState, NewJob};

#[derive(Clone)]
pub struct JobQueue {
    store: Arc<dyn JobStore>,
    bus: EventBus,
    tx: mpsc::UnboundedSender<JobId>,
}

pub struct JobQueueReceiver {
    pub rx: mpsc::UnboundedReceiver<JobId>,
}

impl JobQueue {
    /// Build a new queue. The receiver is handed to the worker(s).
    pub fn new(store: Arc<dyn JobStore>, bus: EventBus) -> (Self, JobQueueReceiver) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Self { store, bus, tx }, JobQueueReceiver { rx })
    }

    /// Submit a NewJob: persist via the store, publish JobCreated, dispatch
    /// to the worker channel.
    pub async fn submit(&self, new: NewJob) -> Result<Job> {
        let job = self.store.create_job(new).await?;
        self.bus
            .publish(Event::JobCreated {
                job: job.id,
                dut: job.dut.clone(),
            })
            .await?;
        // Sender failure means the worker side dropped. Surface it.
        self.tx
            .send(job.id)
            .map_err(|_| crate::error::DaemonError::Config("worker channel closed".into()))?;
        debug!(job = %job.id, "submitted");
        Ok(job)
    }

    /// Update the job state and publish a JobStateChanged event.
    pub async fn transition(&self, id: JobId, state: JobState) -> Result<()> {
        self.store.update_state(id, state.clone()).await?;
        self.bus
            .publish(Event::JobStateChanged { job: id, state })
            .await?;
        Ok(())
    }

    pub fn store(&self) -> Arc<dyn JobStore> {
        self.store.clone()
    }

    pub fn bus(&self) -> &EventBus {
        &self.bus
    }
}

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use super::*;
    use crate::store::SqliteJobStore;
    use crate::types::JobKind;
    use heimdall_core::DutId;

    #[tokio::test]
    async fn submit_persists_emits_dispatches() {
        let store = Arc::new(SqliteJobStore::open_in_memory().await.unwrap()) as Arc<dyn JobStore>;
        let bus = EventBus::new(store.clone(), 16);
        let (queue, mut recv) = JobQueue::new(store.clone(), bus.clone());
        let mut sub = bus.subscribe();

        let job = queue
            .submit(NewJob {
                dut: DutId::new("d1"),
                kind: JobKind::MockHello,
                campaign: None,
            })
            .await
            .unwrap();

        // Event broadcast received.
        let ev = sub.recv().await.unwrap();
        match ev {
            Event::JobCreated { job: j, .. } => assert_eq!(j, job.id),
            other => panic!("unexpected {other:?}"),
        }
        // Dispatch channel received the id.
        let id = recv.rx.recv().await.unwrap();
        assert_eq!(id, job.id);
        // Stored in JobStore.
        let stored = store.get_job(job.id).await.unwrap().unwrap();
        assert_eq!(stored.id, job.id);
    }
}
