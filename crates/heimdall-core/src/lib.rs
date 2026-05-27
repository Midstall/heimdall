//! Heimdall core types: identifiers, artifacts, verdicts, errors.
//! Pure data, no I/O.

pub mod artifact;
pub mod error;
pub mod ids;
pub mod kind;
pub mod observation;
pub mod state;
pub mod stimulus;
pub mod verdict;

pub use artifact::{Artifact, ArtifactKind, BitstreamFormat, Provenance};
pub use error::CoreError;
pub use ids::{DutId, RunId, SeedId};
pub use kind::DutKind;
pub use observation::Observation;
pub use state::{State, ValueRepr};
pub use stimulus::{StepBudget, Stimulus};
pub use verdict::{Evidence, FailureKind, SkipReason, Verdict};

/// Workspace version. Picks up `HEIMDALL_FULL_VERSION` from the build
/// environment when set (Nix), otherwise falls back to `CARGO_PKG_VERSION`.
pub const VERSION: &str = match option_env!("HEIMDALL_FULL_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};
