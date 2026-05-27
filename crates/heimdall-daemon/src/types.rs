use bytes::Bytes;
use chrono::{DateTime, Utc};
use heimdall_core::{DutId, DutKind, Verdict};
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JobId(pub Uuid);

impl JobId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for JobId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for JobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LeaseId(pub Uuid);

impl LeaseId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for LeaseId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EventId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum JobKind {
    /// Runs the built-in "mock-hello" test against the DUT.
    MockHello,
    /// Loads an Aegis bitstream via JTAG. `descriptor_json` carries the
    /// device layout (notably `total_bits`). `bitstream_b64` is the raw
    /// bitstream, base64-encoded.
    LoadAegisBitstream {
        descriptor_json: String,
        bitstream_b64: String,
    },
    /// Load an Aegis bitstream, drive the configured input pads to the
    /// supplied values, settle, then read output pads and diff against the
    /// supplied expected_outputs.
    RunAegisVector {
        descriptor_json: String,
        bitstream_b64: String,
        #[serde(default)]
        inputs: std::collections::BTreeMap<String, bool>,
        #[serde(default)]
        expected_outputs: std::collections::BTreeMap<String, bool>,
        #[serde(default)]
        settle_cycles: u64,
    },
    /// Boots a precompiled River ELF via OpenOCD JTAG. Cycles is the
    /// wait_halt budget in millisecond-equivalents (see RiverCpuDriver::run).
    BootRiverElf { elf_b64: String, cycles: u64 },
    /// Generic. The daemon dispatches based on `name` and `payload`.
    Named {
        name: String,
        payload: serde_json::Value,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "state", content = "detail")]
pub enum JobState {
    Queued,
    Running,
    Done(VerdictSummary),
    Failed(String),
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum VerdictSummary {
    Pass,
    Fail { reason: String },
    Skip { reason: String },
    Error { message: String },
}

impl From<&Verdict> for VerdictSummary {
    fn from(v: &Verdict) -> Self {
        match v {
            Verdict::Pass => Self::Pass,
            Verdict::Fail { kind, .. } => Self::Fail {
                reason: kind.to_string(),
            },
            Verdict::Skip { reason } => Self::Skip {
                reason: format!("{reason:?}"),
            },
            Verdict::Error { message } => Self::Error {
                message: message.clone(),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CampaignId(pub Uuid);

impl CampaignId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for CampaignId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for CampaignId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum CampaignTemplate {
    BringUp,
    Characterization,
    Release,
    Custom { name: String },
}

impl CampaignTemplate {
    pub fn name(&self) -> &str {
        match self {
            Self::BringUp => "bring-up",
            Self::Characterization => "characterization",
            Self::Release => "release",
            Self::Custom { name } => name,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "state")]
pub enum CampaignState {
    Pending,
    Running,
    Pass,
    Fail,
    Mixed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Campaign {
    pub id: CampaignId,
    pub dut: DutId,
    pub chip_serial: Option<String>,
    pub template: CampaignTemplate,
    pub state: CampaignState,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewJob {
    pub dut: DutId,
    pub kind: JobKind,
    #[serde(default)]
    pub campaign: Option<CampaignId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: JobId,
    pub dut: DutId,
    pub dut_kind: DutKind,
    pub kind: JobKind,
    pub campaign: Option<CampaignId>,
    pub state: JobState,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lease {
    pub id: LeaseId,
    pub dut: DutId,
    pub holder: JobId,
    pub acquired_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum Event {
    JobCreated {
        job: JobId,
        dut: DutId,
    },
    JobStateChanged {
        job: JobId,
        state: JobState,
    },
    JobLog {
        job: JobId,
        level: String,
        message: String,
    },
    LeaseAcquired {
        lease: LeaseId,
        dut: DutId,
        holder: JobId,
    },
    LeaseReleased {
        lease: LeaseId,
        dut: DutId,
    },
    CampaignCreated {
        campaign: CampaignId,
        dut: DutId,
        template: CampaignTemplate,
    },
    CampaignStateChanged {
        campaign: CampaignId,
        state: CampaignState,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BlobId(pub String);

impl BlobId {
    pub fn from_bytes(bytes: &[u8]) -> Self {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(bytes);
        Self(hex::encode(h.finalize()))
    }
}

impl fmt::Display for BlobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Default)]
pub struct JobFilter {
    pub dut: Option<DutId>,
    pub state_in: Option<Vec<JobStateTag>>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum JobStateTag {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}

impl JobState {
    pub fn tag(&self) -> JobStateTag {
        match self {
            Self::Queued => JobStateTag::Queued,
            Self::Running => JobStateTag::Running,
            Self::Done(_) => JobStateTag::Done,
            Self::Failed(_) => JobStateTag::Failed,
            Self::Cancelled => JobStateTag::Cancelled,
        }
    }
}

/// A blob stored in the BlobStore. Convenience wrapper.
#[derive(Debug, Clone)]
pub struct Blob {
    pub id: BlobId,
    pub bytes: Bytes,
}
