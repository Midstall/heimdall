use std::sync::Arc;

use async_trait::async_trait;
use clap::Args as ClapArgs;
use eyre::Result;
use heimdall::core::{Artifact, ArtifactKind, DutId, DutKind, State, StepBudget, ValueRepr};
use heimdall::driver::{Dut, MockDriver};
use heimdall::golden::MockGoldenModel;
use heimdall::test::{BuildCtx, Plan, Runner, Stage, StageMessage, StageObserver, Test, TestError};
use indicatif::{ProgressBar, ProgressStyle};

#[derive(Debug, ClapArgs)]
pub struct Args {
    /// Built-in test name. Currently only `mock-hello` is wired.
    #[arg(long, default_value = "mock-hello")]
    pub test: String,
}

struct MockHello;

#[async_trait::async_trait]
impl Test for MockHello {
    fn name(&self) -> &str {
        "mock-hello"
    }
    fn target(&self) -> DutKind {
        DutKind::RiverRc1Nano
    }
    async fn build(&self, _ctx: &mut BuildCtx<'_>) -> Result<Plan, TestError> {
        Ok(Plan {
            input: Artifact::new(ArtifactKind::Asm, &b"li a0, 0x42"[..]),
            expected: State::new().with("x10", ValueRepr::U64(0x42)),
            budget: StepBudget::cycles(1000),
            inputs: std::collections::BTreeMap::new(),
        })
    }
}

/// Wires `Stage` notifications from the runner into an indicatif spinner.
/// Each stage transition advances the bar position and updates the message,
/// so the user sees `prepare`, `compile`, `load`, ... scroll past as they
/// happen instead of just an opaque final verdict line.
struct StageBar {
    bar: ProgressBar,
}

const STAGE_STEPS: u64 = 11;

#[async_trait]
impl StageObserver for StageBar {
    async fn stage(&self, stage: Stage, message: StageMessage) {
        let args: Vec<(&str, &str)> = message.args.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let rendered = heimdall::i18n::t_args(message.key, &args);
        self.bar.set_message(format!("{stage}: {rendered}"));
        self.bar.inc(1);
    }
}

pub async fn run(args: Args, _cfg_path: Option<std::path::PathBuf>) -> Result<()> {
    if args.test != "mock-hello" {
        return Err(eyre::eyre!("only `mock-hello` is supported"));
    }
    let bar = ProgressBar::new(STAGE_STEPS).with_style(
        ProgressStyle::with_template(
            "{spinner:.cyan} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len} {msg}",
        )
        .expect("valid template")
        .progress_chars("=>-"),
    );
    bar.enable_steady_tick(std::time::Duration::from_millis(120));
    let observer: Arc<dyn StageObserver> = Arc::new(StageBar { bar: bar.clone() });
    let runner = Runner::builder().with_observer(observer).build();
    let mut driver = MockDriver::new(DutKind::RiverRc1Nano)
        .with_state(State::new().with("x10", ValueRepr::U64(0x42)));
    let mut golden = MockGoldenModel::new(DutKind::RiverRc1Nano);
    let mut dut = Dut::new(DutId::new("mock-dut"), DutKind::RiverRc1Nano);
    let res = runner
        .run_one(&MockHello, &mut dut, &mut driver, &mut golden)
        .await?;
    bar.finish_with_message(format!("verdict={:?}", res.verdict));
    tracing::info!(verdict = ?res.verdict, elapsed = ?res.elapsed, "run complete");
    if !res.verdict.is_pass() {
        return Err(eyre::eyre!("verdict not pass: {:?}", res.verdict));
    }
    Ok(())
}
