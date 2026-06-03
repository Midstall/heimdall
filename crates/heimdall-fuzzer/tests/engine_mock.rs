//! End-to-end fuzzer integration test using mocks throughout.

use async_trait::async_trait;
use heimdall_core::{
    Artifact, ArtifactKind, DutId, DutKind, Observation, State, StepBudget, Stimulus, ValueRepr,
    Verdict,
};
use heimdall_driver::error::DriverError;
use heimdall_driver::{Dut, MockDriver, TestDriver};
use heimdall_fuzzer::{BitFlipMutator, FuzzerEngine, RawAsmGen, RoundRobinScheduler, Rv64};
use heimdall_golden::MockGoldenModel;
use heimdall_test::Runner;
use heimdall_transport::TransportKind;

#[tokio::test]
async fn fuzz_ten_iterations_against_mocks() {
    // Driver and golden both default to a0=0x42; every iteration's diff Passes.
    let driver = MockDriver::new(DutKind::RiverRc1Nano)
        .with_state(State::new().with("x10", ValueRepr::U64(0x42)));
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

/// Driver whose `run()` always errors. Used to confirm that one bad
/// iteration doesn't take down the whole fuzz session: the engine should
/// catch the error, count it under `errors`, and keep going. Models the
/// real-world case where a randomly-generated program traps or loops
/// forever and the wait_halt timeout propagates a `TransportError` up to
/// the runner.
struct AlwaysErroringDriver {
    target: DutKind,
}

#[async_trait]
impl TestDriver for AlwaysErroringDriver {
    fn target(&self) -> DutKind {
        self.target
    }
    fn required_transports(&self) -> &[TransportKind] {
        &[]
    }
    async fn prepare(&mut self, _dut: &mut Dut) -> Result<(), DriverError> {
        Ok(())
    }
    async fn compile(
        &mut self,
        input: &Artifact,
        _tools: &heimdall_tools::ToolChain,
    ) -> Result<Artifact, DriverError> {
        let mut a = input.clone();
        a.kind = ArtifactKind::ElfRiscv;
        Ok(a)
    }
    async fn load(&mut self, _dut: &mut Dut, _image: &Artifact) -> Result<(), DriverError> {
        Ok(())
    }
    async fn run(&mut self, _dut: &mut Dut, _stim: &Stimulus) -> Result<Observation, DriverError> {
        Err(DriverError::State("simulated wait_halt timeout"))
    }
    async fn observe(&mut self, _dut: &mut Dut) -> Result<State, DriverError> {
        Ok(State::new())
    }
    async fn diff(&self, _dut: &State, _golden: &State) -> Verdict {
        Verdict::Pass
    }
    async fn release(&mut self, _dut: &mut Dut) -> Result<(), DriverError> {
        Ok(())
    }
}

#[tokio::test]
async fn fuzz_continues_past_per_iteration_driver_errors() {
    let driver = AlwaysErroringDriver {
        target: DutKind::RiverRc1Nano,
    };
    let golden = MockGoldenModel::new(DutKind::RiverRc1Nano);
    let mut dut = Dut::new(DutId::new("d-err"), DutKind::RiverRc1Nano);

    let mut engine = FuzzerEngine::builder()
        .with_runner(Runner::builder().build())
        .with_generator(RawAsmGen::<Rv64>::new(4))
        .with_mutator(BitFlipMutator)
        .with_scheduler(RoundRobinScheduler::new())
        .with_driver(driver)
        .with_golden(golden)
        .with_rng_seed(0xfeedface)
        .with_step_budget(StepBudget::cycles(100))
        .build();

    // Engine.run MUST resolve to Ok with errors == iterations, NOT
    // propagate the driver's error out and abort the whole session.
    let report = engine
        .run(&mut dut, 5)
        .await
        .expect("engine should not abort");
    assert_eq!(report.iterations, 5);
    assert_eq!(
        report.errors, 5,
        "every iteration's driver error should land under `errors`"
    );
    assert_eq!(report.passes, 0);
    assert_eq!(report.fails, 0);
}

/// The daemon's web UI shows "what is the fuzzer running right now"
/// by registering a [`heimdall_fuzzer::ProgramCallback`] that copies
/// each iter's artifact bytes into a per-job cache. Pin the contract
/// here so a future refactor that drops the observer call shows up
/// as a unit-test failure rather than a silent regression in the UI.
#[tokio::test]
async fn program_observer_fires_per_iter_with_each_artifact() {
    use std::sync::Arc;
    use std::sync::Mutex;

    let driver = MockDriver::new(DutKind::RiverRc1Nano)
        .with_state(State::new().with("x10", ValueRepr::U64(0)));
    let golden = MockGoldenModel::new(DutKind::RiverRc1Nano)
        .with_state(State::new().with("x10", ValueRepr::U64(0)));
    let mut dut = Dut::new(DutId::new("d-obs"), DutKind::RiverRc1Nano);

    type CapturedIter = (u64, Vec<u8>, ArtifactKind);
    let captured: Arc<Mutex<Vec<CapturedIter>>> = Arc::new(Mutex::new(Vec::new()));
    let captured_clone = captured.clone();
    let observer: heimdall_fuzzer::ProgramCallback = Arc::new(move |iter, artifact: &Artifact| {
        captured_clone
            .lock()
            .unwrap()
            .push((iter, artifact.bytes.to_vec(), artifact.kind.clone()));
    });

    let mut engine = FuzzerEngine::builder()
        .with_runner(Runner::builder().build())
        .with_generator(RawAsmGen::<Rv64>::new(4))
        .with_mutator(BitFlipMutator)
        .with_scheduler(RoundRobinScheduler::new())
        .with_driver(driver)
        .with_golden(golden)
        .with_rng_seed(0x1234_5678)
        .with_step_budget(StepBudget::cycles(100))
        .with_program_observer(observer)
        .build();

    let _ = engine.run(&mut dut, 3).await.expect("run");

    let log = captured.lock().unwrap();
    assert_eq!(log.len(), 3, "observer must fire exactly once per iter");
    // Iter indices are 0..N in order.
    for (i, (iter, _, _)) in log.iter().enumerate() {
        assert_eq!(*iter, i as u64, "iter index must be sequential");
    }
    // All artifacts are RawBytes per the RawAsmGen contract.
    for (_, _, kind) in log.iter() {
        assert!(matches!(kind, ArtifactKind::RawBytes));
    }
    // Each artifact has bytes (prologue + body + ebreak by default).
    for (_, bytes, _) in log.iter() {
        assert!(!bytes.is_empty(), "every iter must surface non-empty bytes");
    }
}
