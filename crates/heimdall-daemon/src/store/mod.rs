use async_trait::async_trait;
use bytes::Bytes;

use crate::error::Result;
use crate::types::{
    BlobId, Campaign, CampaignId, CampaignState, Event, EventId, Job, JobFilter, JobId, JobState,
    NewJob,
};

#[async_trait]
pub trait JobStore: Send + Sync {
    async fn create_job(&self, new: NewJob) -> Result<Job>;
    async fn get_job(&self, id: JobId) -> Result<Option<Job>>;
    async fn list_jobs(&self, filter: JobFilter) -> Result<Vec<Job>>;
    async fn update_state(&self, id: JobId, state: JobState) -> Result<()>;
    async fn append_event(&self, ev: Event) -> Result<EventId>;
    async fn list_events_since(&self, since: EventId, limit: u32) -> Result<Vec<(EventId, Event)>>;
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

    /// Insert an event with its original `EventId`. After import, future
    /// `append_event` calls must produce IDs strictly greater than every
    /// imported one.
    async fn import_event(&self, _id: EventId, _ev: Event) -> Result<()> {
        Err(crate::error::DaemonError::Unsupported(
            "import_event not implemented for this JobStore",
        ))
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
