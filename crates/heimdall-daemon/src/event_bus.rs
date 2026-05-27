//! In-process event bus. Wraps a tokio broadcast channel and also persists
//! events to a JobStore (so the WebSocket replay path can backfill from disk
//! across reconnects).

use std::sync::Arc;

use tokio::sync::broadcast;

use crate::error::Result;
use crate::store::JobStore;
use crate::types::Event;

#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<Event>,
    store: Arc<dyn JobStore>,
}

impl EventBus {
    pub fn new(store: Arc<dyn JobStore>, capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity);
        Self { tx, store }
    }

    /// Publish an event. Persists to the store and emits on the broadcast
    /// channel. Broadcast failure (no subscribers) is fine. Only store
    /// persistence failure is returned.
    pub async fn publish(&self, ev: Event) -> Result<()> {
        let _ = self.store.append_event(ev.clone()).await?;
        // broadcast::Sender::send returns Err if there are no receivers.
        if let Err(broadcast::error::SendError(_)) = self.tx.send(ev) {
            // No live subscribers. The persisted event remains readable
            // via JobStore::list_events_since.
        }
        Ok(())
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.tx.subscribe()
    }

    /// Number of active subscribers (for diagnostics).
    pub fn receiver_count(&self) -> usize {
        self.tx.receiver_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{EventId, JobId, JobState};

    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn publish_persists_and_broadcasts() {
        use crate::store::SqliteJobStore;
        let store = Arc::new(SqliteJobStore::open_in_memory().await.unwrap()) as Arc<dyn JobStore>;
        let bus = EventBus::new(store.clone(), 16);
        let mut rx = bus.subscribe();
        let job = JobId::new();
        let ev = Event::JobStateChanged {
            job,
            state: JobState::Running,
        };
        bus.publish(ev.clone()).await.unwrap();
        let recv = rx.recv().await.unwrap();
        match recv {
            Event::JobStateChanged { job: j, state } => {
                assert_eq!(j, job);
                assert!(matches!(state, JobState::Running));
            }
            other => panic!("unexpected event: {other:?}"),
        }
        let stored = store.list_events_since(EventId(0), 10).await.unwrap();
        assert_eq!(stored.len(), 1);
    }
}
