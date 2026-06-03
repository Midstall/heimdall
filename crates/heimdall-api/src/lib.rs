//! Wire types for the Heimdall daemon's HTTP + WebSocket surface.
//!
//! Pure data crate: every public item is a `serde`-friendly enum or
//! struct describing the on-the-wire shape an external client sees.
//! No server logic, no I/O, no driver internals. That's what the
//! `heimdall-daemon` crate is for.
//!
//! Keeping the wire types in their own crate lets us:
//!
//! - Generate client bindings for any language (TypeScript today
//!   via `ts-rs`, with Python, Go, etc. dropping in alongside as new
//!   features on this crate) without dragging the daemon's runtime
//!   deps into the codegen step.
//! - Sidestep the build-graph cycle the bindings used to have:
//!   `heimdall-daemon -> heimdall-web` (for the embedded asset
//!   bundle) meant `heimdall-web/build.rs` couldn't depend on
//!   `heimdall-daemon` to read the types it needed to render. With
//!   `heimdall-api` as a thin leaf both crates can depend on, the
//!   web crate's build.rs reads the types directly and emits the
//!   `.ts` files on every cargo build, no commit needed.
//!
//! Conversion impls that bridge into runtime-side types
//! (`heimdall_core::Verdict` -> [`VerdictSummary`],
//! `heimdall_test::SnapshotSource` -> [`SnapshotSourceTag`]) live in
//! `heimdall-daemon` so this crate stays a leaf.

use bytes::Bytes;
use chrono::{DateTime, Utc};
use heimdall_core::{DutId, DutKind};
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
#[cfg_attr(feature = "bindings", derive(ts_rs::TS), ts(export))]
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
    /// Coverage-guided fuzz session against a configured DUT. The factory
    /// (`RiverFuzzFactory`, etc.) builds the concrete driver+golden out
    /// of the DUT registry. The engine itself runs `iterations` rounds and
    /// reports the aggregate verdict.
    Fuzz {
        /// Number of fuzz iterations to run.
        iterations: u64,
        /// RNG seed. Deterministic with the same seed + generator config.
        seed: u64,
        /// Instructions per generated program (for `RawAsmGen`-style
        /// generators).
        #[serde(default = "default_fuzz_insn_count")]
        insn_count: usize,
        /// Per-iteration wait_halt budget in cycle-equivalents. Passed
        /// straight to the driver via `Stimulus { budget: Cycles { count } }`.
        #[serde(default = "default_fuzz_cycles")]
        cycles: u64,
        /// If true, a sim-vs-silicon coverage divergence flips the verdict
        /// to fail. Off by default so fuzz runs keep going even when the
        /// silicon and golden disagree on coverage.
        #[serde(default)]
        strict_coverage: bool,
        /// Which code generator drives this fuzz session. Default
        /// `raw-asm` keeps the proven hand-rolled RV64 encoder. The
        /// `cranelift` choice requires the daemon (and heimdall-fuzzer)
        /// to be built with the `cranelift` feature. The worker
        /// surfaces a clean error otherwise.
        #[serde(default)]
        generator: GeneratorKind,
    },
    /// Generic. The daemon dispatches based on `name` and `payload`.
    Named {
        name: String,
        // The TypeScript binding renders the payload as `unknown` so the
        // generated `JobKind.ts` doesn't need to import a cross-crate
        // `JsonValue.ts` (ts-rs's serde-json-impl writes that into
        // each crate's local `./bindings/` directory, producing a
        // brittle relative path).
        #[cfg_attr(feature = "bindings", ts(type = "unknown"))]
        payload: serde_json::Value,
    },
}

fn default_fuzz_insn_count() -> usize {
    16
}
fn default_fuzz_cycles() -> u64 {
    1_000
}

/// Which code generator a `JobKind::Fuzz` session uses. `RawAsm` is
/// the always-available hand-rolled RV64 encoder. `Cranelift` lowers
/// a randomised SSA IR through Cranelift's RV64 backend and only
/// works when the daemon was built with the `cranelift` cargo
/// feature (which transitively enables `heimdall-fuzzer/cranelift`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS), ts(export))]
pub enum GeneratorKind {
    #[default]
    RawAsm,
    Cranelift,
}

impl JobKind {
    /// Short identifier for logs and UI badges. Matches the serde tag, so the
    /// same string the wire payload uses also names the variant in human-
    /// readable contexts.
    pub fn tag(&self) -> &'static str {
        match self {
            JobKind::MockHello => "mock-hello",
            JobKind::LoadAegisBitstream { .. } => "load-aegis-bitstream",
            JobKind::RunAegisVector { .. } => "run-aegis-vector",
            JobKind::BootRiverElf { .. } => "boot-river-elf",
            JobKind::Fuzz { .. } => "fuzz",
            JobKind::Named { .. } => "named",
        }
    }

    /// One-line summary suitable for a `JobLog` message body. Includes the
    /// variant tag plus only the small scalar parameters, never the bulky
    /// base64 / JSON-blob fields. The bulky fields would otherwise wrap
    /// across hundreds of columns in the web UI and TUI.
    pub fn summary(&self) -> String {
        match self {
            JobKind::MockHello => "mock-hello".to_string(),
            JobKind::LoadAegisBitstream { bitstream_b64, .. } => {
                let bytes = b64_decoded_len(bitstream_b64);
                format!("load-aegis-bitstream (bitstream={bytes}B)")
            }
            JobKind::RunAegisVector {
                bitstream_b64,
                inputs,
                expected_outputs,
                settle_cycles,
                ..
            } => {
                let bytes = b64_decoded_len(bitstream_b64);
                format!(
                    "run-aegis-vector (bitstream={bytes}B, inputs={}, expect={}, cycles={settle_cycles})",
                    inputs.len(),
                    expected_outputs.len(),
                )
            }
            JobKind::BootRiverElf { elf_b64, cycles } => {
                let bytes = b64_decoded_len(elf_b64);
                format!("boot-river-elf (elf={bytes}B, cycles={cycles})")
            }
            JobKind::Fuzz {
                iterations,
                seed,
                insn_count,
                cycles,
                strict_coverage,
                generator,
            } => {
                let strict = if *strict_coverage { ", strict" } else { "" };
                let gen_label = match generator {
                    GeneratorKind::RawAsm => "raw-asm",
                    GeneratorKind::Cranelift => "cranelift",
                };
                format!(
                    "fuzz (iters={iterations}, seed=0x{seed:x}, insns={insn_count}, cycles={cycles}, gen={gen_label}{strict})"
                )
            }
            JobKind::Named { name, .. } => format!("named {name}"),
        }
    }
}

/// Estimate the decoded length of a base64 string without actually decoding.
/// Off by up to 2 bytes when the trailing padding is missing or atypical, but
/// good enough for a log summary.
fn b64_decoded_len(s: &str) -> usize {
    let raw = s.trim().len();
    if raw == 0 {
        return 0;
    }
    let pad = s.bytes().rev().take_while(|&b| b == b'=').count();
    (raw * 3) / 4 - pad
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "state", content = "detail")]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS), ts(export))]
pub enum JobState {
    Queued,
    Running,
    Done(VerdictSummary),
    Failed(String),
    Cancelled,
    /// Terminal state for jobs that were `Running` when the daemon
    /// process exited (crash, SIGTERM, restart). The reconcile pass
    /// in `runtime::start_inner` walks the store at boot and flips
    /// any orphaned `Running` row to `Dead` so the UI and operators
    /// can tell "the daemon went away under this job" apart from a
    /// clean `Failed` or operator `Cancelled`.
    Dead,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS), ts(export))]
pub enum VerdictSummary {
    Pass,
    Fail { reason: String },
    Skip { reason: String },
    Error { message: String },
}

impl From<&heimdall_core::Verdict> for VerdictSummary {
    fn from(v: &heimdall_core::Verdict) -> Self {
        match v {
            heimdall_core::Verdict::Pass => Self::Pass,
            heimdall_core::Verdict::Fail { kind, .. } => Self::Fail {
                reason: kind.to_string(),
            },
            heimdall_core::Verdict::Skip { reason } => Self::Skip {
                reason: format!("{reason:?}"),
            },
            heimdall_core::Verdict::Error { message } => Self::Error {
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
#[cfg_attr(feature = "bindings", derive(ts_rs::TS), ts(export))]
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
#[cfg_attr(feature = "bindings", derive(ts_rs::TS), ts(export))]
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

/// Wire-side tag mirroring `heimdall_test::SnapshotSource`. Lives here so
/// the `Event::DutStateSnapshot` variant has a Serialize/Deserialize-safe
/// kebab-case tag without the daemon needing a serde impl on the test
/// crate's enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS), ts(export))]
pub enum SnapshotSourceTag {
    Dut,
    Golden,
}

/// Severity tag on a `JobLog`. Matches the tracing convention. Serialized
/// as lowercase strings ("info", "warn", ...) for the WebSocket wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS), ts(export))]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            LogLevel::Trace => "trace",
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
        }
    }

    /// i18n catalog key for the level's display label.
    pub fn i18n_key(self) -> &'static str {
        match self {
            LogLevel::Trace => "log.level.trace",
            LogLevel::Debug => "log.level.debug",
            LogLevel::Info => "log.level.info",
            LogLevel::Warn => "log.level.warn",
            LogLevel::Error => "log.level.error",
        }
    }
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Wire envelope wrapping an [`Event`] with the UTC timestamp the daemon
/// assigned at publish time. Backend always stores UTC. Clients localize on
/// display. Uses `#[serde(flatten)]` so the JSON shape stays
/// `{"ts": "...", "kind": "...", ...event-fields}` rather than nesting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StampedEvent {
    pub ts: DateTime<Utc>,
    #[serde(flatten)]
    pub event: Event,
}

impl StampedEvent {
    pub fn new(ts: DateTime<Utc>, event: Event) -> Self {
        Self { ts, event }
    }
}

/// One row returned by `JobStore::list_events_since` and
/// `JobStore::list_job_logs`: the storage id, the assigned UTC timestamp,
/// and the event itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EventRecord {
    pub id: EventId,
    pub ts: DateTime<Utc>,
    #[serde(flatten)]
    pub event: Event,
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
        level: LogLevel,
        /// Server-rendered fallback. Whenever `i18n_key` is set, clients
        /// should prefer localizing the key+args themselves.
        message: String,
        /// Optional stage tag (kebab-case Stage variant). `None` for
        /// daemon-level logs that don't correspond to a Runner stage.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stage: Option<String>,
        /// Stable i18n catalog key. Together with `i18n_args`, lets each
        /// client render the message in its own locale rather than the
        /// daemon's. Absent on free-form ad-hoc logs.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        i18n_key: Option<String>,
        /// Named-argument substitutions for `i18n_key`. Values are
        /// pre-stringified so consumers never have to parse typed args.
        #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
        i18n_args: std::collections::BTreeMap<String, String>,
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
    /// Architectural snapshot captured during the runner's `observe()`
    /// phase. The DUT card mirrors the latest snapshot per DUT so users
    /// can peek at registers / PC without halting the CPU again.
    DutStateSnapshot {
        dut: DutId,
        job: JobId,
        source: SnapshotSourceTag,
        state: heimdall_core::State,
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
    /// Row offset for pagination. `0` (or `None`) returns the first
    /// page. Combined with `limit` this gives the web UI's
    /// Prev/Next pager what it needs without a cursor.
    pub offset: Option<u32>,
    /// Drop jobs created strictly older than this. Used by the
    /// prune surface to scope a delete to "everything completed
    /// before X." `None` = no cutoff.
    pub created_before: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum JobStateTag {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
    Dead,
}

impl JobState {
    pub fn tag(&self) -> JobStateTag {
        match self {
            Self::Queued => JobStateTag::Queued,
            Self::Running => JobStateTag::Running,
            Self::Done(_) => JobStateTag::Done,
            Self::Failed(_) => JobStateTag::Failed,
            Self::Cancelled => JobStateTag::Cancelled,
            Self::Dead => JobStateTag::Dead,
        }
    }
}

/// Runtime introspection payload exposed by `GET /about`. Used by
/// the web "About" tab and the TUI's about view to render the
/// running daemon's identity + which optional features it was built
/// with. Sourced from `cfg!(feature = "...")` at daemon compile
/// time, so reflects the actual binary on disk rather than whatever
/// the wire client thinks should be enabled.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS), ts(export))]
pub struct AboutInfo {
    /// Crate version of `heimdall-core`. Matches the value baked
    /// into other surfaces (`/health`, `/metrics`).
    pub version: String,
    /// Build profile of the daemon binary: `"debug"` or `"release"`.
    pub build_profile: String,
    /// Cargo feature flags compiled into the daemon. The set is
    /// derived from `cfg!(feature = ...)` at build time.
    pub features: AboutFeatures,
}

/// Per-feature presence flags for [`AboutInfo`]. Each field maps
/// 1:1 to a Cargo feature in `crates/heimdall-daemon/Cargo.toml`.
/// `true` means the daemon binary was built with that feature on.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS), ts(export))]
pub struct AboutFeatures {
    pub sqlite: bool,
    pub aegis: bool,
    pub river: bool,
    pub fuzzer: bool,
    pub cranelift: bool,
}

/// A blob stored in the BlobStore. Convenience wrapper.
#[derive(Debug, Clone)]
pub struct Blob {
    pub id: BlobId,
    pub bytes: Bytes,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_omits_bulky_base64_payloads() {
        let big = "A".repeat(4096); // 4 KiB of base64, ~3 KiB decoded.
        let k = JobKind::BootRiverElf {
            elf_b64: big.clone(),
            cycles: 5000,
        };
        let s = k.summary();
        assert!(
            !s.contains(&big),
            "summary should not contain raw base64; got `{s}`"
        );
        assert!(s.contains("boot-river-elf"), "got `{s}`");
        assert!(s.contains("cycles=5000"), "got `{s}`");
        assert!(s.contains("elf=3072B"), "decoded len wrong; got `{s}`");
        assert!(s.len() < 80, "summary should stay short; got {}B", s.len());
    }

    #[test]
    fn summary_handles_each_variant() {
        assert_eq!(JobKind::MockHello.summary(), "mock-hello");
        assert!(
            JobKind::LoadAegisBitstream {
                descriptor_json: "{}".into(),
                bitstream_b64: "AAAA".into(),
            }
            .summary()
            .starts_with("load-aegis-bitstream")
        );
        assert!(
            JobKind::RunAegisVector {
                descriptor_json: "{}".into(),
                bitstream_b64: "AAAA".into(),
                inputs: std::collections::BTreeMap::new(),
                expected_outputs: std::collections::BTreeMap::new(),
                settle_cycles: 1,
            }
            .summary()
            .contains("run-aegis-vector")
        );
        assert_eq!(
            JobKind::Named {
                name: "n".into(),
                payload: serde_json::Value::Null,
            }
            .summary(),
            "named n"
        );
    }

    #[test]
    fn tag_matches_serde_tag() {
        assert_eq!(JobKind::MockHello.tag(), "mock-hello");
        assert_eq!(
            JobKind::BootRiverElf {
                elf_b64: "".into(),
                cycles: 0,
            }
            .tag(),
            "boot-river-elf"
        );
    }
}
