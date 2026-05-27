//! Coverage-guided fuzzing against a mock DUT + mock golden. No hardware needed.
//!
//! Spins up the full fuzzer engine (generator + mutator + scheduler + Runner +
//! differential oracle) with all-mock backends, runs N iterations, and prints
//! the resulting `FuzzReport`.
//!
//! Run with:
//! ```sh
//! cargo run -p heimdall-fuzzer --example mock_fuzz
//! cargo run -p heimdall-fuzzer --example mock_fuzz -- 200
//! ```
//!
//! Note: the mock driver's silicon-coverage source intentionally uses a
//! different hash salt than the mock golden's sim-coverage so that the
//! divergence-detection path is exercised by tests. Expect a non-zero
//! `divergences` count here; it is not a real bug.

use heimdall_core::{DutId, DutKind, State, StepBudget, ValueRepr};
use heimdall_driver::{Dut, MockDriver};
use heimdall_fuzzer::{BitFlipMutator, FuzzerEngine, RawAsmGen, RoundRobinScheduler, Rv64};
use heimdall_golden::MockGoldenModel;
use heimdall_test::Runner;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_target(false)
        .init();

    let iterations: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(100);

    let driver = MockDriver::new(DutKind::RiverRc1Nano)
        .with_state(State::new().with("a0", ValueRepr::U64(0x42)));
    let golden = MockGoldenModel::new(DutKind::RiverRc1Nano);
    let mut dut = Dut::new(DutId::new("mock-1"), DutKind::RiverRc1Nano);

    let mut engine = FuzzerEngine::builder()
        .with_runner(Runner::builder().build())
        .with_generator(RawAsmGen::<Rv64>::new(8))
        .with_mutator(BitFlipMutator)
        .with_scheduler(RoundRobinScheduler::new())
        .with_driver(driver)
        .with_golden(golden)
        .with_rng_seed(0xC0FFEE)
        .with_step_budget(StepBudget::cycles(100))
        .build();

    let report = engine
        .run(&mut dut, iterations)
        .await
        .expect("fuzzer should not error against mocks");

    println!();
    println!("iterations  : {}", report.iterations);
    println!("passes      : {}", report.passes);
    println!("fails       : {}", report.fails);
    println!("errors      : {}", report.errors);
    println!("corpus size : {}", report.corpus_size);
    println!("sim bits    : {}", report.coverage_bits);
    println!("silicon bits: {}", report.silicon_coverage_bits);
    println!("divergences : {}", report.divergences.len());
}
