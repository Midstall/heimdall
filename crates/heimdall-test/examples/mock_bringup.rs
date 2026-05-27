//! End-to-end mock bringup. No hardware needed.
//!
//! Composes a `MockDriver` with a `MockGoldenModel`, runs a tiny RISC-V
//! "hello" stimulus through the full compile/prepare/load/run/observe/diff
//! pipeline, and prints the verdict.
//!
//! Run with:
//! ```sh
//! cargo run -p heimdall-test --example mock_bringup
//! ```

use async_trait::async_trait;
use heimdall_core::{Artifact, ArtifactKind, DutId, DutKind, State, StepBudget, ValueRepr};
use heimdall_driver::{Dut, MockDriver};
use heimdall_golden::MockGoldenModel;
use heimdall_test::{BuildCtx, Plan, Runner, Test, TestError};

struct HelloTest;

#[async_trait]
impl Test for HelloTest {
    fn name(&self) -> &str {
        "hello"
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

#[tokio::main(flavor = "current_thread")]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_target(false)
        .init();

    let runner = Runner::builder().build();
    let mut driver = MockDriver::new(DutKind::RiverRc1Nano)
        .with_state(State::new().with("a0", ValueRepr::U64(0x42)));
    let mut golden = MockGoldenModel::new(DutKind::RiverRc1Nano);
    let mut dut = Dut::new(DutId::new("d1"), DutKind::RiverRc1Nano);

    let result = runner
        .run_one(&HelloTest, &mut dut, &mut driver, &mut golden)
        .await
        .expect("mock bringup should not error");

    println!();
    println!("run id  : {}", result.run_id);
    println!("verdict : {:?}", result.verdict);
    println!("elapsed : {:?}", result.elapsed);
    if let Some(obs) = result.observation {
        println!("observed: {:?}", obs.state);
    }
}
