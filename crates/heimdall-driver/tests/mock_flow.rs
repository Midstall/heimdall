use heimdall_core::{
    Artifact, ArtifactKind, DutId, DutKind, State, StepBudget, Stimulus, ValueRepr, Verdict,
};
use heimdall_driver::{Dut, MockDriver, TestDriver};
use heimdall_tools::ToolChain;

#[tokio::test]
async fn full_mock_flow_passes() {
    let mut driver = MockDriver::new(DutKind::RiverRc1Nano)
        .with_state(State::new().with("a0", ValueRepr::U64(0x42)));
    let mut dut = Dut::new(DutId::new("d1"), DutKind::RiverRc1Nano);
    let input = Artifact::new(ArtifactKind::Asm, &b"li a0, 0x42"[..]);
    let tools = ToolChain::new();

    driver.prepare(&mut dut).await.unwrap();
    let img = driver.compile(&input, &tools).await.unwrap();
    driver.load(&mut dut, &img).await.unwrap();
    let _obs = driver
        .run(&mut dut, &Stimulus::new(StepBudget::cycles(100)))
        .await
        .unwrap();
    let dut_state = driver.observe(&mut dut).await.unwrap();
    let golden_state = State::new().with("a0", ValueRepr::U64(0x42));
    let v = driver.diff(&dut_state, &golden_state).await;
    assert!(matches!(v, Verdict::Pass));
    driver.release(&mut dut).await.unwrap();
}

#[tokio::test]
async fn diff_mismatch_reports_field() {
    let driver = MockDriver::new(DutKind::RiverRc1Nano);
    let dut = State::new().with("a0", ValueRepr::U64(41));
    let golden = State::new().with("a0", ValueRepr::U64(42));
    let v = driver.diff(&dut, &golden).await;
    match v {
        Verdict::Fail { kind, .. } => {
            assert!(format!("{kind}").contains("a0"));
        }
        _ => panic!("expected fail"),
    }
}
