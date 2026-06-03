//! `JobLogger` wraps an [`EventBus`] and a [`JobId`] so the test runner can
//! emit per-stage events that get fanned out over the `/events` WebSocket.
//! Surfaces in the web UI as a live job timeline.

use std::collections::BTreeMap;

use async_trait::async_trait;
use heimdall_core::{DutId, State};
use heimdall_i18n::Locale;
use heimdall_test::{SnapshotSource, Stage, StageMessage, StageObserver};
use tracing::warn;

use crate::event_bus::EventBus;
use crate::types::{Event, JobId, LogLevel, SnapshotSourceTag};

#[derive(Clone)]
pub struct JobLogger {
    bus: EventBus,
    job: JobId,
}

impl JobLogger {
    pub fn new(bus: EventBus, job: JobId) -> Self {
        Self { bus, job }
    }

    /// Publish a daemon-level `JobLog` keyed by an i18n catalog entry.
    /// Each named arg is stringified once on the publish side. Used by the
    /// worker for its own dispatch-summary line.
    pub async fn log_keyed(
        &self,
        level: LogLevel,
        key: &'static str,
        args: &[(&'static str, String)],
    ) {
        let arg_refs: Vec<(&str, &str)> = args.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let message = heimdall_i18n::t_args_in(Locale::En, key, &arg_refs);
        let i18n_args: BTreeMap<String, String> = args
            .iter()
            .map(|(k, v)| ((*k).to_string(), v.clone()))
            .collect();
        self.publish(level, message, None, Some(key.to_string()), i18n_args)
            .await;
    }

    /// Publish a daemon-level `JobLog` with a free-form English message
    /// (no catalog key). Reserved for ad-hoc messages that don't yet have
    /// a translation key, e.g. propagated driver error strings.
    pub async fn log(&self, level: LogLevel, message: impl Into<String>) {
        self.publish(level, message.into(), None, None, BTreeMap::new())
            .await;
    }

    async fn publish(
        &self,
        level: LogLevel,
        message: String,
        stage: Option<Stage>,
        i18n_key: Option<String>,
        i18n_args: BTreeMap<String, String>,
    ) {
        if let Err(e) = self
            .bus
            .publish(Event::JobLog {
                job: self.job,
                level,
                message,
                stage: stage.map(|s| s.as_str().to_string()),
                i18n_key,
                i18n_args,
            })
            .await
        {
            warn!(error = %e, job = %self.job, "job-log publish failed");
        }
    }
}

#[async_trait]
impl StageObserver for JobLogger {
    async fn stage(&self, stage: Stage, message: StageMessage) {
        let arg_refs: Vec<(&str, &str)> =
            message.args.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let rendered = heimdall_i18n::t_args_in(Locale::En, message.key, &arg_refs);
        let i18n_args: BTreeMap<String, String> = message
            .args
            .iter()
            .map(|(k, v)| ((*k).to_string(), v.clone()))
            .collect();
        self.publish(
            LogLevel::Info,
            rendered,
            Some(stage),
            Some(message.key.to_string()),
            i18n_args,
        )
        .await;
    }

    async fn state_snapshot(&self, source: SnapshotSource, dut: &DutId, state: &State) {
        if let Err(e) = self
            .bus
            .publish(Event::DutStateSnapshot {
                dut: dut.clone(),
                job: self.job,
                source: SnapshotSourceTag::from(source),
                state: state.clone(),
            })
            .await
        {
            warn!(error = %e, job = %self.job, "dut-state-snapshot publish failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn stage_publishes_job_log_event_with_i18n_key_and_args() {
        use crate::store::{JobStore, SqliteJobStore};
        let store = Arc::new(SqliteJobStore::open_in_memory().await.unwrap()) as Arc<dyn JobStore>;
        let bus = EventBus::new(store.clone(), 16);
        let mut rx = bus.subscribe();
        let job = JobId::new();
        let logger = JobLogger::new(bus, job);
        logger
            .stage(
                Stage::Load,
                StageMessage::new("log.runner.load_start").arg("bytes", 30),
            )
            .await;
        let stamped = rx.recv().await.unwrap();
        match stamped.event {
            Event::JobLog {
                job: j,
                level,
                message,
                stage,
                i18n_key,
                i18n_args,
            } => {
                assert_eq!(j, job);
                assert_eq!(level, LogLevel::Info);
                // Server-rendered English fallback uses the en catalog.
                assert_eq!(message, "loading image into dut (30 bytes)");
                assert_eq!(stage.as_deref(), Some("load"));
                assert_eq!(i18n_key.as_deref(), Some("log.runner.load_start"));
                assert_eq!(i18n_args.get("bytes").map(String::as_str), Some("30"));
            }
            other => panic!("unexpected event {other:?}"),
        }
    }

    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn log_publishes_without_stage_tag() {
        use crate::store::{JobStore, SqliteJobStore};
        let store = Arc::new(SqliteJobStore::open_in_memory().await.unwrap()) as Arc<dyn JobStore>;
        let bus = EventBus::new(store.clone(), 16);
        let mut rx = bus.subscribe();
        let job = JobId::new();
        let logger = JobLogger::new(bus, job);
        logger.log(LogLevel::Info, "dispatching mock-hello").await;
        let stamped = rx.recv().await.unwrap();
        match stamped.event {
            Event::JobLog {
                stage, i18n_key, ..
            } => {
                assert!(stage.is_none());
                assert!(i18n_key.is_none());
            }
            other => panic!("unexpected event {other:?}"),
        }
    }

    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn log_keyed_sets_i18n_fields() {
        use crate::store::{JobStore, SqliteJobStore};
        let store = Arc::new(SqliteJobStore::open_in_memory().await.unwrap()) as Arc<dyn JobStore>;
        let bus = EventBus::new(store.clone(), 16);
        let mut rx = bus.subscribe();
        let job = JobId::new();
        let logger = JobLogger::new(bus, job);
        logger
            .log_keyed(
                LogLevel::Info,
                "log.worker.dispatching",
                &[("summary", "boot-river-elf (elf=1216B, cycles=2000)".into())],
            )
            .await;
        let stamped = rx.recv().await.unwrap();
        match stamped.event {
            Event::JobLog {
                message,
                i18n_key,
                i18n_args,
                ..
            } => {
                assert_eq!(i18n_key.as_deref(), Some("log.worker.dispatching"));
                assert_eq!(
                    i18n_args.get("summary").map(String::as_str),
                    Some("boot-river-elf (elf=1216B, cycles=2000)"),
                );
                assert!(message.starts_with("dispatching "));
            }
            other => panic!("unexpected event {other:?}"),
        }
    }
}
