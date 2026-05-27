use clap::Args as ClapArgs;
use eyre::Result;
use heimdall::core::{Artifact, ArtifactKind, DutId, DutKind, State, StepBudget, ValueRepr};
use heimdall::driver::{Dut, MockDriver};
use heimdall::golden::MockGoldenModel;
use heimdall::test::{BuildCtx, Plan, Runner, Test, TestError};

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
            expected: State::new().with("a0", ValueRepr::U64(0x42)),
            budget: StepBudget::cycles(1000),
            inputs: std::collections::BTreeMap::new(),
        })
    }
}

pub async fn run(args: Args, _cfg_path: Option<std::path::PathBuf>) -> Result<()> {
    if args.test != "mock-hello" {
        return Err(eyre::eyre!("only `mock-hello` is supported"));
    }
    let runner = Runner::builder().build();
    let mut driver = MockDriver::new(DutKind::RiverRc1Nano)
        .with_state(State::new().with("a0", ValueRepr::U64(0x42)));
    let mut golden = MockGoldenModel::new(DutKind::RiverRc1Nano);
    let mut dut = Dut::new(DutId::new("mock-dut"), DutKind::RiverRc1Nano);
    let res = runner
        .run_one(&MockHello, &mut dut, &mut driver, &mut golden)
        .await?;
    tracing::info!(verdict = ?res.verdict, elapsed = ?res.elapsed, "run complete");
    if !res.verdict.is_pass() {
        return Err(eyre::eyre!("verdict not pass: {:?}", res.verdict));
    }
    Ok(())
}
