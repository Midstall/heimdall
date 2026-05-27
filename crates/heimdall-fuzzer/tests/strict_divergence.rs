//! Verifies strict_coverage mode flips passing iterations into Failed
//! whenever sim and silicon coverage diverge.

use heimdall_core::{DutId, DutKind, State, StepBudget, ValueRepr};
use heimdall_driver::{Dut, MockDriver};
use heimdall_fuzzer::{BitFlipMutator, FuzzerEngine, PowerScheduler, RawAsmGen, Rv64};
use heimdall_golden::MockGoldenModel;
use heimdall_test::Runner;

#[tokio::test]
async fn strict_coverage_flips_pass_to_fail_on_divergence() {
    // Setup matches coverage_growth.rs but uses strict_coverage = true.
    // Mock and MockDriver intentionally use different salts so every
    // iteration produces a divergent snapshot; strict mode therefore
    // converts every passing iteration into a Failed one.
    let driver = MockDriver::new(DutKind::RiverRc1Nano)
        .with_state(State::new().with("a0", ValueRepr::U64(0x42)));
    let golden = MockGoldenModel::new(DutKind::RiverRc1Nano);
    let mut dut = Dut::new(DutId::new("d1"), DutKind::RiverRc1Nano);

    let mut engine = FuzzerEngine::builder()
        .with_runner(Runner::builder().build())
        .with_generator(RawAsmGen::<Rv64>::new(4))
        .with_mutator(BitFlipMutator)
        .with_scheduler(PowerScheduler::new())
        .with_driver(driver)
        .with_golden(golden)
        .with_rng_seed(0xc0ffee)
        .with_step_budget(StepBudget::cycles(100))
        .with_strict_coverage(true)
        .build();

    let report = engine.run(&mut dut, 5).await.expect("run");
    assert_eq!(report.iterations, 5);
    // Strict mode: every iteration that diverged is counted as fail.
    assert!(
        report.fails > 0,
        "expected strict mode to convert divergent passes into fails; got fails={}",
        report.fails
    );
    assert_eq!(
        report.passes + report.fails + report.errors + report.skips,
        report.iterations
    );
    // Divergence findings are recorded regardless of strict mode.
    assert!(!report.divergences.is_empty());
}

#[tokio::test]
async fn non_strict_keeps_pass_verdict_despite_divergence() {
    let driver = MockDriver::new(DutKind::RiverRc1Nano)
        .with_state(State::new().with("a0", ValueRepr::U64(0x42)));
    let golden = MockGoldenModel::new(DutKind::RiverRc1Nano);
    let mut dut = Dut::new(DutId::new("d1"), DutKind::RiverRc1Nano);

    let mut engine = FuzzerEngine::builder()
        .with_runner(Runner::builder().build())
        .with_generator(RawAsmGen::<Rv64>::new(4))
        .with_mutator(BitFlipMutator)
        .with_scheduler(PowerScheduler::new())
        .with_driver(driver)
        .with_golden(golden)
        .with_rng_seed(0xc0ffee)
        .with_step_budget(StepBudget::cycles(100))
        .with_strict_coverage(false)
        .build();

    let report = engine.run(&mut dut, 5).await.expect("run");
    // Without strict mode, passing iterations stay passing even when divergent.
    assert!(report.passes > 0, "non-strict run should report passes");
    // Divergence findings should still be recorded (sanity).
    assert!(!report.divergences.is_empty());
}
