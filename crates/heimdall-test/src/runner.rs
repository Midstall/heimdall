use async_trait::async_trait;
use heimdall_core::{DutId, Observation, RunId, State, Stimulus, Verdict};
use heimdall_driver::{Dut, TestDriver};
use heimdall_golden::GoldenModel;
use heimdall_tools::ToolChain;
use std::sync::Arc;
use std::time::Instant;
use tracing::instrument;

use crate::error::TestError;
use crate::test_trait::{BuildCtx, Test};

/// Where a `state_snapshot` observation came from. Surfaces in the wire
/// `Event::DutStateSnapshot` so UIs can label the registers source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotSource {
    /// State captured via `TestDriver::observe` (live silicon or sim).
    Dut,
    /// State captured via `GoldenModel::observe`.
    Golden,
}

impl SnapshotSource {
    pub fn as_str(self) -> &'static str {
        match self {
            SnapshotSource::Dut => "dut",
            SnapshotSource::Golden => "golden",
        }
    }
}

/// Project the runner-side capture source onto the wire-side tag
/// the daemon publishes via `Event::DutStateSnapshot`. Lives here
/// (not in `heimdall-api`) because `heimdall-api` must stay a leaf
/// for `heimdall-web/build.rs` to consume it; the impl needs the
/// `SnapshotSource` Local + the foreign `SnapshotSourceTag` to
/// satisfy the orphan rule, which only works from this side.
impl From<SnapshotSource> for heimdall_api::SnapshotSourceTag {
    fn from(s: SnapshotSource) -> Self {
        match s {
            SnapshotSource::Dut => heimdall_api::SnapshotSourceTag::Dut,
            SnapshotSource::Golden => heimdall_api::SnapshotSourceTag::Golden,
        }
    }
}

/// One step in the per-job lifecycle, ordered by where the Runner emits it.
/// Values are stable identifiers so the web UI can sort/render by stage,
/// not just by wall-clock order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stage {
    Build,
    Compile,
    Prepare,
    GoldenReset,
    GoldenLoad,
    Load,
    Run,
    GoldenStep,
    Observe,
    Diff,
    Release,
}

impl Stage {
    /// Stable kebab-case identifier suitable for JSON / kebab-case
    /// serialization on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            Stage::Build => "build",
            Stage::Compile => "compile",
            Stage::Prepare => "prepare",
            Stage::GoldenReset => "golden-reset",
            Stage::GoldenLoad => "golden-load",
            Stage::Load => "load",
            Stage::Run => "run",
            Stage::GoldenStep => "golden-step",
            Stage::Observe => "observe",
            Stage::Diff => "diff",
            Stage::Release => "release",
        }
    }

    /// i18n catalog key for this stage's display label. Clients pass it
    /// through `heimdall_i18n::t()` (or the equivalent in the web UI's
    /// fetched `/i18n.json`) to localize the badge in the UI.
    pub fn i18n_key(self) -> &'static str {
        match self {
            Stage::Build => "log.stage.build",
            Stage::Compile => "log.stage.compile",
            Stage::Prepare => "log.stage.prepare",
            Stage::GoldenReset => "log.stage.golden-reset",
            Stage::GoldenLoad => "log.stage.golden-load",
            Stage::Load => "log.stage.load",
            Stage::Run => "log.stage.run",
            Stage::GoldenStep => "log.stage.golden-step",
            Stage::Observe => "log.stage.observe",
            Stage::Diff => "log.stage.diff",
            Stage::Release => "log.stage.release",
        }
    }
}

/// Returned by `<Stage as FromStr>::from_str` when the tag string
/// isn't one of the known kebab-case stage names. Carries the
/// offending input so error messages can surface what was seen.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown stage tag `{0}`")]
pub struct UnknownStage(pub String);

impl std::str::FromStr for Stage {
    type Err = UnknownStage;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "build" => Stage::Build,
            "compile" => Stage::Compile,
            "prepare" => Stage::Prepare,
            "golden-reset" => Stage::GoldenReset,
            "golden-load" => Stage::GoldenLoad,
            "load" => Stage::Load,
            "run" => Stage::Run,
            "golden-step" => Stage::GoldenStep,
            "observe" => Stage::Observe,
            "diff" => Stage::Diff,
            "release" => Stage::Release,
            other => return Err(UnknownStage(other.to_string())),
        })
    }
}

impl std::fmt::Display for Stage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One stage notification. Carries a stable i18n catalog key so each client
/// can localize per its locale, plus named arguments for placeholder
/// substitution. Bring your own catalog: the runner only emits the key.
#[derive(Debug, Clone)]
pub struct StageMessage {
    pub key: &'static str,
    pub args: Vec<(&'static str, String)>,
}

impl StageMessage {
    pub fn new(key: &'static str) -> Self {
        Self {
            key,
            args: Vec::new(),
        }
    }

    pub fn arg(mut self, name: &'static str, value: impl ToString) -> Self {
        self.args.push((name, value.to_string()));
        self
    }
}

/// Pluggable observer that the Runner calls once per stage transition. The
/// daemon implements this to fan stage events out over its EventBus so the
/// web UI can render a live per-job timeline. The Runner ignores observer
/// errors so a stuck publish never blocks the test.
#[async_trait]
pub trait StageObserver: Send + Sync {
    async fn stage(&self, stage: Stage, message: StageMessage);

    /// Called once per `observe()` so observers can broadcast or cache the
    /// captured architectural state. Default impl ignores it, so existing
    /// observers stay source-compatible.
    async fn state_snapshot(&self, _source: SnapshotSource, _dut: &DutId, _state: &State) {}
}

pub struct Runner {
    pub tools: ToolChain,
    observer: Option<Arc<dyn StageObserver>>,
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

        self.notify(Stage::Build, StageMessage::new("log.runner.build_start"))
            .await;
        let mut ctx = BuildCtx {
            target: driver.target(),
            _marker: std::marker::PhantomData,
        };
        let plan = test.build(&mut ctx).await?;

        self.notify(
            Stage::Compile,
            StageMessage::new("log.runner.compile_start"),
        )
        .await;
        let image = driver.compile(&plan.input, &self.tools).await?;

        self.notify(
            Stage::Prepare,
            StageMessage::new("log.runner.prepare_start"),
        )
        .await;
        driver.prepare(dut).await?;
        self.notify(
            Stage::GoldenReset,
            StageMessage::new("log.runner.golden_reset"),
        )
        .await;
        golden.reset().await?;
        self.notify(
            Stage::GoldenLoad,
            StageMessage::new("log.runner.golden_load"),
        )
        .await;
        golden.load(&image).await?;

        self.notify(
            Stage::Load,
            StageMessage::new("log.runner.load_start").arg("bytes", image.bytes.len()),
        )
        .await;
        driver.load(dut, &image).await?;
        self.notify(
            Stage::Run,
            StageMessage::new("log.runner.run_start").arg("budget", format_budget(&plan.budget)),
        )
        .await;
        let observation = driver
            .run(
                dut,
                &Stimulus {
                    budget: plan.budget,
                    inputs: plan.inputs.clone(),
                },
            )
            .await?;
        self.notify(
            Stage::GoldenStep,
            StageMessage::new("log.runner.golden_step"),
        )
        .await;
        let _ = golden.step(plan.budget).await?;

        self.notify(Stage::Observe, StageMessage::new("log.runner.observe_dut"))
            .await;
        let dut_state = driver.observe(dut).await?;
        self.notify_state(SnapshotSource::Dut, &dut.id, &dut_state)
            .await;
        self.notify(
            Stage::Observe,
            StageMessage::new("log.runner.observe_golden"),
        )
        .await;
        let golden_state = golden.observe().await?;
        self.notify_state(SnapshotSource::Golden, &dut.id, &golden_state)
            .await;

        let v_vs_expected = driver.diff(&dut_state, &plan.expected).await;
        let v_vs_golden = driver.diff(&dut_state, &golden_state).await;

        let verdict = match (&v_vs_expected, &v_vs_golden) {
            (Verdict::Pass, Verdict::Pass) => Verdict::Pass,
            (Verdict::Pass, fail @ Verdict::Fail { .. }) => fail.clone(),
            (fail @ Verdict::Fail { .. }, _) => fail.clone(),
            _ => Verdict::Pass,
        };
        self.notify(
            Stage::Diff,
            StageMessage::new("log.runner.diff_result").arg("verdict", format!("{verdict:?}")),
        )
        .await;

        driver.release(dut).await?;
        self.notify(Stage::Release, StageMessage::new("log.runner.release_done"))
            .await;

        Ok(RunResult {
            run_id,
            verdict,
            observation: Some(observation),
            elapsed: started.elapsed(),
        })
    }

    async fn notify(&self, stage: Stage, message: StageMessage) {
        if let Some(o) = &self.observer {
            o.stage(stage, message).await;
        }
    }

    async fn notify_state(&self, source: SnapshotSource, dut: &DutId, state: &State) {
        if let Some(o) = &self.observer {
            o.state_snapshot(source, dut, state).await;
        }
    }
}

/// Format a `StepBudget` as a short kebab-y string for log args. Stays
/// catalog-agnostic: the same string flows through every locale.
fn format_budget(budget: &heimdall_core::StepBudget) -> String {
    match budget {
        heimdall_core::StepBudget::Cycles { count } => format!("cycles={count}"),
        heimdall_core::StepBudget::Duration { millis } => format!("ms={millis}"),
    }
}

#[derive(Default)]
pub struct RunnerBuilder {
    tools: ToolChain,
    observer: Option<Arc<dyn StageObserver>>,
}

impl RunnerBuilder {
    pub fn with_tools(mut self, tools: ToolChain) -> Self {
        self.tools = tools;
        self
    }

    pub fn with_observer(mut self, observer: Arc<dyn StageObserver>) -> Self {
        self.observer = Some(observer);
        self
    }

    pub fn build(self) -> Runner {
        Runner {
            tools: self.tools,
            observer: self.observer,
        }
    }
}
