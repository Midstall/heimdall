use heimdall_core::{Artifact, ArtifactKind, DutKind, State, StepBudget, ValueRepr};
use heimdall_golden::{GoldenError, GoldenModel, MockGoldenModel, StepOutcome};

#[tokio::test]
async fn load_then_observe() {
    let mut g = MockGoldenModel::new(DutKind::RiverRc1Nano);
    let img = Artifact::new(ArtifactKind::ElfRiscv, &b"fake-elf"[..]);
    g.load(&img).await.unwrap();
    let out = g.step(StepBudget::cycles(100)).await.unwrap();
    assert_eq!(out, StepOutcome::RanFully);
    let state = g.observe().await.unwrap();
    assert_eq!(state.fields.get("a0"), Some(&ValueRepr::U64(0x42)));
}

#[tokio::test]
async fn step_before_load_errors() {
    let mut g = MockGoldenModel::new(DutKind::RiverRc1Nano);
    let err = g.step(StepBudget::cycles(1)).await.unwrap_err();
    assert!(matches!(err, GoldenError::NotLoaded));
}

#[tokio::test]
async fn override_state() {
    let s = State::new().with("a1", ValueRepr::U64(7));
    let mut g = MockGoldenModel::new(DutKind::RiverRc1Nano).with_state(s);
    let img = Artifact::new(ArtifactKind::ElfRiscv, &[][..]);
    g.load(&img).await.unwrap();
    let out = g.observe().await.unwrap();
    assert_eq!(out.fields.get("a1"), Some(&ValueRepr::U64(7)));
}
