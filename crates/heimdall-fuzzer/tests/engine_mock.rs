//! End-to-end fuzzer integration test using mocks throughout.

use heimdall_core::{DutId, DutKind, State, StepBudget, ValueRepr};
use heimdall_driver::{Dut, MockDriver};
use heimdall_fuzzer::{BitFlipMutator, FuzzerEngine, RawAsmGen, RoundRobinScheduler, Rv64};
use heimdall_golden::MockGoldenModel;
use heimdall_test::Runner;

#[tokio::test]
async fn fuzz_ten_iterations_against_mocks() {
    // Driver and golden both default to a0=0x42; every iteration's diff Passes.
    let driver = MockDriver::new(DutKind::RiverRc1Nano)
        .with_state(State::new().with("a0", ValueRepr::U64(0x42)));
    let golden = MockGoldenModel::new(DutKind::RiverRc1Nano);
    let mut dut = Dut::new(DutId::new("d1"), DutKind::RiverRc1Nano);

    let mut engine = FuzzerEngine::builder()
        .with_runner(Runner::builder().build())
        .with_generator(RawAsmGen::<Rv64>::new(8))
        .with_mutator(BitFlipMutator)
        .with_scheduler(RoundRobinScheduler::new())
        .with_driver(driver)
        .with_golden(golden)
        .with_rng_seed(0xdeadbeef)
        .with_step_budget(StepBudget::cycles(100))
        .build();

    let report = engine.run(&mut dut, 10).await.expect("run");
    assert_eq!(report.iterations, 10);
    assert_eq!(
        report.passes, 10,
        "all iterations should pass against matching mocks"
    );
    assert_eq!(report.errors, 0);
    assert!(report.corpus_size > 0, "corpus should grow");
}
