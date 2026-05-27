use crate::state::ValueRepr;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    pub label: String,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SkipReason {
    NotApplicable,
    Cosmetic,
}

/// Typed failure kinds. NEVER add a String-reason variant; thread information
/// through structured fields.
#[derive(Debug, Clone, Error, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum FailureKind {
    #[error("diff mismatch on field `{field}`: got {got}, expected {expected}")]
    DiffMismatch {
        field: String,
        got: ValueRepr,
        expected: ValueRepr,
    },
    #[error("dut unresponsive for at least {millis} ms")]
    DutUnresponsive { millis: u64 },
    #[error("timed out after {elapsed_ms} ms of {budget_ms} ms budget")]
    Timeout { budget_ms: u64, elapsed_ms: u64 },
    #[error("golden model error: {detail}")]
    GoldenError { detail: String },
    #[error("bad stimulus: {detail}")]
    BadStimulus { detail: String },
    #[error("coverage divergence in bucket `{bucket}`")]
    CoverageDivergence { bucket: String },
}

impl FailureKind {
    pub fn dut_unresponsive(d: Duration) -> Self {
        Self::DutUnresponsive {
            millis: d.as_millis() as u64,
        }
    }

    pub fn timeout(budget: Duration, elapsed: Duration) -> Self {
        Self::Timeout {
            budget_ms: budget.as_millis() as u64,
            elapsed_ms: elapsed.as_millis() as u64,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Verdict {
    Pass,
    Fail {
        kind: FailureKind,
        evidence: Vec<Evidence>,
    },
    Skip {
        reason: SkipReason,
    },
    Error {
        message: String,
    },
}

impl Verdict {
    pub fn is_pass(&self) -> bool {
        matches!(self, Self::Pass)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_mismatch_display() {
        let k = FailureKind::DiffMismatch {
            field: "a0".into(),
            got: ValueRepr::U64(41),
            expected: ValueRepr::U64(42),
        };
        let s = k.to_string();
        assert!(s.contains("a0"));
        assert!(s.contains("0x29"));
        assert!(s.contains("0x2a"));
    }

    #[test]
    fn verdict_is_pass() {
        assert!(Verdict::Pass.is_pass());
        assert!(
            !Verdict::Skip {
                reason: SkipReason::Cosmetic
            }
            .is_pass()
        );
    }
}
