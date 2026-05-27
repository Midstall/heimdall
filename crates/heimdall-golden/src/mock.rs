use async_trait::async_trait;
use heimdall_core::{Artifact, DutKind, State, StepBudget, ValueRepr};

use crate::error::GoldenError;
use crate::trait_def::{CoverageSource, GoldenModel, Result, StepOutcome};

/// Deterministic coverage bitmap derived from the keys of a State's fields.
/// Each key is hashed into a fixed-size bitmap.
pub struct MockCoverage {
    bits: Vec<u8>,
}

impl MockCoverage {
    pub fn buckets() -> usize {
        1024
    }

    pub fn from_state(state: &State) -> Self {
        use std::hash::{Hash, Hasher};
        let mut bits = vec![0u8; Self::buckets()];
        for key in state.fields.keys() {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            key.hash(&mut h);
            let v = h.finish();
            let idx = (v as usize) & (Self::buckets() * 8 - 1);
            let byte = idx / 8;
            let bit = (idx % 8) as u8;
            bits[byte] |= 1 << bit;
        }
        Self { bits }
    }
}

impl CoverageSource for MockCoverage {
    fn snapshot(&self) -> Vec<u8> {
        self.bits.clone()
    }
}

pub struct MockGoldenModel {
    target: DutKind,
    pub fixed_state: State,
    pub loaded: bool,
    pub last_outcome: StepOutcome,
    last_coverage: Option<MockCoverage>,
}

impl MockGoldenModel {
    pub fn new(target: DutKind) -> Self {
        let fixed_state = State::new().with("a0", ValueRepr::U64(0x42));
        Self {
            target,
            fixed_state,
            loaded: false,
            last_outcome: StepOutcome::RanFully,
            last_coverage: None,
        }
    }

    pub fn with_state(mut self, state: State) -> Self {
        self.fixed_state = state;
        self
    }
}

#[async_trait]
impl GoldenModel for MockGoldenModel {
    fn target(&self) -> DutKind {
        self.target
    }
    async fn reset(&mut self) -> Result<()> {
        self.loaded = false;
        self.last_coverage = None;
        Ok(())
    }
    async fn load(&mut self, _image: &Artifact) -> Result<()> {
        self.loaded = true;
        Ok(())
    }
    async fn step(&mut self, _budget: StepBudget) -> Result<StepOutcome> {
        if !self.loaded {
            return Err(GoldenError::NotLoaded);
        }
        self.last_coverage = Some(MockCoverage::from_state(&self.fixed_state));
        Ok(self.last_outcome.clone())
    }
    async fn observe(&mut self) -> Result<State> {
        if !self.loaded {
            return Err(GoldenError::NotLoaded);
        }
        Ok(self.fixed_state.clone())
    }
    fn coverage(&self) -> Option<&dyn CoverageSource> {
        self.last_coverage
            .as_ref()
            .map(|c| c as &dyn CoverageSource)
    }
}
