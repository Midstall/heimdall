//! In-process event bus. Wraps a tokio broadcast channel and also persists
//! events to a JobStore (so the WebSocket replay path can backfill from disk
//! across reconnects).

use std::sync::Arc;

use chrono::Utc;
use tokio::sync::broadcast;

use crate::error::Result;
use crate::store::JobStore;
use crate::types::{Event, StampedEvent};

#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<StampedEvent>,
    store: Arc<dyn JobStore>,
}

impl EventBus {
    pub fn new(store: Arc<dyn JobStore>, capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity);
        Self { tx, store }
    }

    /// Publish an event. Stamps a single UTC timestamp, persists via
    /// `append_event_at` so the stored row carries the same value the
    /// broadcast frame reports, and emits on the broadcast channel.
    /// Broadcast failure (no subscribers) is fine. Only store persistence
    /// failure is returned.
    pub async fn publish(&self, ev: Event) -> Result<()> {
        let ts = Utc::now();
        let _ = self.store.append_event_at(ev.clone(), ts).await?;
        let stamped = StampedEvent::new(ts, ev);
        if let Err(broadcast::error::SendError(_)) = self.tx.send(stamped) {
            // No live subscribers. The persisted event remains readable
            // via JobStore::list_events_since.
        }
        Ok(())
    }

    pub fn subscribe(&self) -> broadcast::Receiver<StampedEvent> {
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
        match recv.event {
            Event::JobStateChanged { job: j, state } => {
                assert_eq!(j, job);
                assert!(matches!(state, JobState::Running));
            }
            other => panic!("unexpected event: {other:?}"),
        }
        let stored = store.list_events_since(EventId(0), 10).await.unwrap();
        assert_eq!(stored.len(), 1);
        // Bus and storage agree on the timestamp value: the broadcast frame
        // and the persisted row see the same `ts`.
        assert_eq!(recv.ts, stored[0].ts);
    }
}
