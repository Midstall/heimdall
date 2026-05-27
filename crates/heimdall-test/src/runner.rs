use heimdall_core::{Observation, RunId, Stimulus, Verdict};
use heimdall_driver::{Dut, TestDriver};
use heimdall_golden::GoldenModel;
use heimdall_tools::ToolChain;
use std::time::Instant;
use tracing::instrument;

use crate::error::TestError;
use crate::test_trait::{BuildCtx, Test};

pub struct Runner {
    pub tools: ToolChain,
}

#[derive(Debug)]
pub struct RunResult {
    pub run_id: RunId,
    pub verdict: Verdict,
    pub observation: Option<Observation>,
    pub elapsed: std::time::Duration,
}

impl Runner {
    pub fn builder() -> RunnerBuilder {
        RunnerBuilder::default()
    }

    #[instrument(skip(self, test, dut, driver, golden), fields(test = test.name(), dut = %dut.id))]
    pub async fn run_one(
        &self,
        test: &dyn Test,
        dut: &mut Dut,
        driver: &mut dyn TestDriver,
        golden: &mut dyn GoldenModel,
    ) -> Result<RunResult, TestError> {
        let started = Instant::now();
        let run_id = RunId::new();

        // Build the test plan.
        let mut ctx = BuildCtx {
            target: driver.target(),
            _marker: std::marker::PhantomData,
        };
        let plan = test.build(&mut ctx).await?;

        // Compile via the driver (driver decides which tools).
        let image = driver.compile(&plan.input, &self.tools).await?;

        // Prepare DUT and golden in sequence (golden init is cheap, DUT prep is slow).
        driver.prepare(dut).await?;
        golden.reset().await?;
        golden.load(&image).await?;

        // Run both, then diff.
        driver.load(dut, &image).await?;
        let observation = driver
            .run(
                dut,
                &Stimulus {
                    budget: plan.budget,
                    inputs: plan.inputs.clone(),
                },
            )
            .await?;
        let _ = golden.step(plan.budget).await?;

        let dut_state = driver.observe(dut).await?;
        let golden_state = golden.observe().await?;

        // Diff in two stages: against the test's hard-coded expectation, then
        // against the golden. The test expectation is the "this is what the
        // golden SHOULD say" anchor. The golden is the dynamic reality.
        let v_vs_expected = driver.diff(&dut_state, &plan.expected).await;
        let v_vs_golden = driver.diff(&dut_state, &golden_state).await;

        let verdict = match (&v_vs_expected, &v_vs_golden) {
            (Verdict::Pass, Verdict::Pass) => Verdict::Pass,
            (Verdict::Pass, fail @ Verdict::Fail { .. }) => fail.clone(),
            (fail @ Verdict::Fail { .. }, _) => fail.clone(),
            _ => Verdict::Pass,
        };

        driver.release(dut).await?;

        Ok(RunResult {
            run_id,
            verdict,
            observation: Some(observation),
            elapsed: started.elapsed(),
        })
    }
}

#[derive(Default)]
pub struct RunnerBuilder {
    tools: ToolChain,
}

impl RunnerBuilder {
    pub fn with_tools(mut self, tools: ToolChain) -> Self {
        self.tools = tools;
        self
    }

    pub fn build(self) -> Runner {
        Runner { tools: self.tools }
    }
}
