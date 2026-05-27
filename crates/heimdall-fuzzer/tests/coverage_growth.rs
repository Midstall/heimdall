//! Verifies the FuzzerEngine collects coverage from the MockGoldenModel
//! and reports a non-zero coverage_bits in FuzzReport.

use heimdall_core::{DutId, DutKind, State, StepBudget, ValueRepr};
use heimdall_driver::{Dut, MockDriver};
use heimdall_fuzzer::{BitFlipMutator, FuzzerEngine, PowerScheduler, RawAsmGen, Rv64};
use heimdall_golden::MockGoldenModel;
use heimdall_test::Runner;

#[tokio::test]
async fn fuzz_run_reports_coverage_bits() {
    // MockGoldenModel.coverage hashes the keys of fixed_state.fields into a
    // bitmap; with a single key "a0" we expect at least one bit set after the
    // first step (and steady-state across iterations since the state is
    // constant). The test verifies coverage_bits is > 0, not necessarily
    // growing every iter.
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
        .build();

    let report = engine.run(&mut dut, 6).await.expect("run");
    assert_eq!(report.iterations, 6);
    assert!(
        report.coverage_bits > 0,
        "expected coverage_bits > 0; got {} (passes={}, fails={})",
        report.coverage_bits,
        report.passes,
        report.fails
    );
    assert!(
        report.silicon_coverage_bits > 0,
        "expected silicon_coverage_bits > 0; got {}",
        report.silicon_coverage_bits
    );
    assert!(
        !report.divergences.is_empty(),
        "Mock sim and silicon use different salts so every iter should produce a DivergenceFinding; got 0"
    );
    let first = &report.divergences[0];
    assert!(first.sim_only_bits > 0 || first.silicon_only_bits > 0);
}
