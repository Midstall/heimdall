use async_trait::async_trait;
use heimdall_core::{
    Artifact, ArtifactKind, DutId, DutKind, State, StepBudget, ValueRepr, Verdict,
};
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

#[tokio::test]
async fn mock_end_to_end_pass() {
    let runner = Runner::builder().build();
    let mut driver = MockDriver::new(DutKind::RiverRc1Nano)
        .with_state(State::new().with("a0", ValueRepr::U64(0x42)));
    let mut golden = MockGoldenModel::new(DutKind::RiverRc1Nano);
    let mut dut = Dut::new(DutId::new("d1"), DutKind::RiverRc1Nano);
    let res = runner
        .run_one(&HelloTest, &mut dut, &mut driver, &mut golden)
        .await
        .unwrap();
    assert!(matches!(res.verdict, Verdict::Pass));
}

#[tokio::test]
async fn mock_end_to_end_fail_when_states_diverge() {
    let runner = Runner::builder().build();
    let mut driver = MockDriver::new(DutKind::RiverRc1Nano)
        .with_state(State::new().with("a0", ValueRepr::U64(0xbad)));
    let mut golden = MockGoldenModel::new(DutKind::RiverRc1Nano);
    let mut dut = Dut::new(DutId::new("d1"), DutKind::RiverRc1Nano);
    let res = runner
        .run_one(&HelloTest, &mut dut, &mut driver, &mut golden)
        .await
        .unwrap();
    match res.verdict {
        Verdict::Fail { .. } => {}
        v => panic!("expected fail, got {:?}", v),
    }
}
