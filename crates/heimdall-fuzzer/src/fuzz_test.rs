//! AdHocFuzzTest: a Test that yields a precomputed Artifact and empty expected
//! State. The Runner's diff against the golden is what reports findings.

use async_trait::async_trait;
use heimdall_core::{Artifact, DutKind, State, StepBudget};

use heimdall_test::{BuildCtx, Plan, Test, TestError};

pub struct AdHocFuzzTest {
    pub name: String,
    pub target: DutKind,
    pub artifact: Artifact,
    pub budget: StepBudget,
}

impl AdHocFuzzTest {
    pub fn new(
        name: impl Into<String>,
        target: DutKind,
        artifact: Artifact,
        budget: StepBudget,
    ) -> Self {
        Self {
            name: name.into(),
            target,
            artifact,
            budget,
        }
    }
}

#[async_trait]
impl Test for AdHocFuzzTest {
    fn name(&self) -> &str {
        &self.name
    }
    fn target(&self) -> DutKind {
        self.target
    }
    async fn build(&self, _ctx: &mut BuildCtx<'_>) -> Result<Plan, TestError> {
        Ok(Plan {
            input: self.artifact.clone(),
            expected: State::new(),
            budget: self.budget,
            inputs: std::collections::BTreeMap::new(),
        })
    }
}
