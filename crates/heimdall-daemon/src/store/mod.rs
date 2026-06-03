use async_trait::async_trait;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use heimdall_core::{ArtifactKind, DutKind};

use crate::error::Result;
use crate::types::{
    BlobId, Campaign, CampaignId, CampaignState, Event, EventId, EventRecord, Job, JobFilter,
    JobId, JobState, NewJob,
};

/// Durable pointer to the most-recent program bytes loaded onto a
/// DUT for a given job. The actual bytes live in a [`BlobStore`]
/// under `blob_id`; this struct just carries the routing metadata
/// the disasm route needs to render the response (artifact kind +
/// optional iter index for fuzz jobs).
#[derive(Debug, Clone)]
pub struct JobProgramRef {
    pub blob_id: BlobId,
    pub kind: ArtifactKind,
    pub iter: Option<u64>,
}

#[async_trait]
pub trait JobStore: Send + Sync {
    async fn create_job(&self, new: NewJob, dut_kind: DutKind) -> Result<Job>;
    async fn get_job(&self, id: JobId) -> Result<Option<Job>>;
    async fn list_jobs(&self, filter: JobFilter) -> Result<Vec<Job>>;
    /// Count jobs matching `filter` (ignoring `limit`/`offset`).
    /// The web UI's pager uses this to render "page X of N" and the
    /// prune dialog uses it to surface "this would delete K rows."
    async fn count_jobs(&self, filter: JobFilter) -> Result<u64>;
    async fn update_state(&self, id: JobId, state: JobState) -> Result<()>;
    /// Delete a single terminal job and its associated rows
    /// (events, JobLog entries, the job_programs mapping). Returns
    /// `true` if a row was deleted, `false` if no such job existed.
    /// Refuses non-terminal jobs (caller surfaces a 400) so we don't
    /// drop a row out from under an in-flight worker.
    async fn delete_job(&self, id: JobId) -> Result<bool>;
    /// Bulk-delete every job matching `filter`. Only terminal jobs
    /// are removed; `Queued` / `Running` matches are silently
    /// skipped. Returns the number of rows actually deleted. The
    /// prune route at the daemon level pipes
    /// `JobFilter { created_before: Some(..), state_in: Some(..), .. }`
    /// in here so operators can drop "every fuzz fail older than 7
    /// days" in one shot.
    async fn delete_jobs(&self, filter: JobFilter) -> Result<u64>;

    /// Persist an event using the caller-supplied UTC timestamp. This is
    /// the canonical write path: the [`EventBus`] stamps `Utc::now()` once
    /// and threads the same value into both the broadcast frame and storage,
    /// so wire-observed timestamps match what's in the DB.
    async fn append_event_at(&self, ev: Event, ts: DateTime<Utc>) -> Result<EventId>;

    /// Convenience: stamp `Utc::now()` and call [`Self::append_event_at`].
    /// Library callers that don't need the storage timestamp to match a
    /// previously-broadcast value should prefer this.
    async fn append_event(&self, ev: Event) -> Result<EventId> {
        self.append_event_at(ev, Utc::now()).await
    }

    async fn list_events_since(&self, since: EventId, limit: u32) -> Result<Vec<EventRecord>>;
    /// Return `Event::JobLog` rows for a single job, oldest-first, with
    /// `id > since`. Used by the `/jobs/:id/logs` HTTP route to backfill
    /// history when a client opens a job detail. Implementations should
    /// push the filter down into storage rather than scanning all events.
    async fn list_job_logs(
        &self,
        job: JobId,
        since: EventId,
        limit: u32,
    ) -> Result<Vec<EventRecord>>;
    async fn create_campaign(&self, campaign: Campaign) -> Result<Campaign>;
    async fn get_campaign(&self, id: CampaignId) -> Result<Option<Campaign>>;
    async fn list_campaigns(&self, limit: Option<u32>) -> Result<Vec<Campaign>>;
    async fn update_campaign_state(&self, id: CampaignId, state: CampaignState) -> Result<()>;
    async fn list_jobs_for_campaign(&self, id: CampaignId) -> Result<Vec<Job>>;

    /// Insert a complete `Job` with its existing `JobId`. Used by dump/restore
    /// to preserve cross-references. Default impl errors with `Unsupported`.
    async fn import_job(&self, _job: Job) -> Result<()> {
        Err(crate::error::DaemonError::Unsupported(
            "import_job not implemented for this JobStore",
        ))
    }

    /// Insert a complete `Campaign` with its existing `CampaignId`.
    async fn import_campaign(&self, _campaign: Campaign) -> Result<()> {
        Err(crate::error::DaemonError::Unsupported(
            "import_campaign not implemented for this JobStore",
        ))
    }

    /// Insert an event with its original `EventId` and original UTC
    /// timestamp. After import, future `append_event` calls must produce
    /// IDs strictly greater than every imported one.
    async fn import_event(&self, _id: EventId, _ts: DateTime<Utc>, _ev: Event) -> Result<()> {
        Err(crate::error::DaemonError::Unsupported(
            "import_event not implemented for this JobStore",
        ))
    }

    /// Upsert "the most recent program bytes for this job" mapping.
    /// The bytes themselves are written to the `BlobStore` separately
    /// (the caller uploads, gets a `BlobId`, then hands the id here).
    /// One row per job; subsequent calls overwrite, so the cache
    /// always reflects the latest iter for fuzz workloads.
    ///
    /// Default impl is `Unsupported` so backends without a key-value
    /// surface (memory-only stores) keep compiling.
    async fn set_job_program(&self, _job: JobId, _program: JobProgramRef) -> Result<()> {
        Err(crate::error::DaemonError::Unsupported(
            "set_job_program not implemented for this JobStore",
        ))
    }

    /// Fetch the persisted program reference for a job, or `None`
    /// when the job hasn't recorded one yet. Used by the disasm
    /// route as a fallback when the in-memory `LoadedProgramCache`
    /// is cold (typically right after a daemon restart).
    async fn get_job_program(&self, _job: JobId) -> Result<Option<JobProgramRef>> {
        Ok(None)
    }
}

#[async_trait]
pub trait BlobStore: Send + Sync {
    async fn put(&self, bytes: &[u8]) -> Result<BlobId>;
    async fn get(&self, id: &BlobId) -> Result<Option<Bytes>>;
    async fn exists(&self, id: &BlobId) -> Result<bool>;

    /// Enumerate all blob ids currently in the store. Used by dump/restore.
    /// Default impl errors with `Unsupported` so non-listable backends (e.g.
    /// a write-only S3 implementation) need not provide it.
    async fn list_ids(&self) -> Result<Vec<BlobId>> {
        Err(crate::error::DaemonError::Unsupported(
            "list_ids not implemented for this BlobStore",
        ))
    }
}

pub mod local_fs;

pub use local_fs::LocalFsBlobStore;

#[cfg(feature = "sqlite")]
pub mod sqlite;

#[cfg(feature = "sqlite")]
pub use sqlite::SqliteJobStore;
