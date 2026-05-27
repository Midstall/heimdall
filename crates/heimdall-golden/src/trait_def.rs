use async_trait::async_trait;
use heimdall_core::{Artifact, DutKind, State, StepBudget};

use crate::error::GoldenError;

pub type Result<T> = std::result::Result<T, GoldenError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepOutcome {
    RanFully,
    HitBudget,
    Trapped,
}

pub trait CoverageSource: Send + Sync {
    fn snapshot(&self) -> Vec<u8>;
}

#[async_trait]
pub trait GoldenModel: Send + Sync {
    fn target(&self) -> DutKind;
    async fn reset(&mut self) -> Result<()>;
    async fn load(&mut self, image: &Artifact) -> Result<()>;
    async fn step(&mut self, budget: StepBudget) -> Result<StepOutcome>;
    async fn observe(&mut self) -> Result<State>;
    fn coverage(&self) -> Option<&dyn CoverageSource> {
        None
    }
}
