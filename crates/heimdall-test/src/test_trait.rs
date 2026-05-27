use async_trait::async_trait;
use heimdall_core::{Artifact, DutKind, State, StepBudget};
use std::collections::BTreeMap;

use crate::error::TestError;

pub struct BuildCtx<'a> {
    pub target: DutKind,
    pub _marker: std::marker::PhantomData<&'a ()>,
}

#[derive(Debug, Clone)]
pub struct Plan {
    pub input: Artifact,
    pub expected: State,
    pub budget: StepBudget,
    /// Named input drives forwarded to Stimulus when the runner invokes the
    /// driver's run method. Empty for tests that do not drive pads.
    #[allow(dead_code)]
    pub inputs: BTreeMap<String, bool>,
}

impl Plan {
    pub fn new(input: Artifact, expected: State, budget: StepBudget) -> Self {
        Self {
            input,
            expected,
            budget,
            inputs: BTreeMap::new(),
        }
    }

    pub fn with_inputs(mut self, inputs: BTreeMap<String, bool>) -> Self {
        self.inputs = inputs;
        self
    }
}

#[async_trait]
pub trait Test: Send + Sync {
    fn name(&self) -> &str;
    fn target(&self) -> DutKind;
    async fn build(&self, ctx: &mut BuildCtx<'_>) -> Result<Plan, TestError>;
}
