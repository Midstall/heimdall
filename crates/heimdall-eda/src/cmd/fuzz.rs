use clap::Args as ClapArgs;
use eyre::Result;
use heimdall::core::{DutId, DutKind, State, StepBudget, ValueRepr};
use heimdall::driver::{Dut, MockDriver};
use heimdall::fuzzer::{BitFlipMutator, FuzzerEngine, RawAsmGen, RoundRobinScheduler, Rv64};
use heimdall::golden::MockGoldenModel;
use heimdall::test::Runner;

#[derive(Debug, ClapArgs)]
pub struct Args {
    /// Target DUT kind. Currently only `river-rc1-nano` is wired.
    #[arg(long, default_value = "river-rc1-nano")]
    pub target: String,

    /// Number of fuzzing iterations.
    #[arg(long, default_value_t = 50)]
    pub iterations: u64,

    /// RNG seed for deterministic runs.
    #[arg(long, default_value_t = 0xdeadbeef)]
    pub seed: u64,

    /// Number of RV64 instructions per generated input.
    #[arg(long, default_value_t = 16)]
    pub insn_count: usize,

    /// Step budget (cycles) per iteration.
    #[arg(long, default_value_t = 1000)]
    pub cycles: u64,
}

pub async fn run(args: Args, _cfg_path: Option<std::path::PathBuf>) -> Result<()> {
    let target = parse_target(&args.target)?;
    tracing::info!(target = ?target, iterations = args.iterations, seed = args.seed, "fuzz start");

    let driver = MockDriver::new(target).with_state(State::new().with("a0", ValueRepr::U64(0x42)));
    let golden = MockGoldenModel::new(target);
    let mut dut = Dut::new(DutId::new("mock-dut"), target);

    let mut engine = FuzzerEngine::builder()
        .with_runner(Runner::builder().build())
        .with_generator(RawAsmGen::<Rv64>::new(args.insn_count))
        .with_mutator(BitFlipMutator)
        .with_scheduler(RoundRobinScheduler::new())
        .with_driver(driver)
        .with_golden(golden)
        .with_rng_seed(args.seed)
        .with_step_budget(StepBudget::cycles(args.cycles))
        .build();

    let report = engine.run(&mut dut, args.iterations).await?;
    tracing::info!(
        iterations = report.iterations,
        passes = report.passes,
        fails = report.fails,
        errors = report.errors,
        corpus_size = report.corpus_size,
        coverage_bits = report.coverage_bits,
        silicon_coverage_bits = report.silicon_coverage_bits,
        elapsed_ms = report.elapsed.as_millis() as u64,
        "fuzz complete"
    );
    if report.errors > 0 {
        return Err(eyre::eyre!(
            "fuzz reported {} infra errors; see logs",
            report.errors
        ));
    }
    Ok(())
}

fn parse_target(s: &str) -> Result<DutKind> {
    match s {
        "aegis-luna1" => Ok(DutKind::AegisLuna1),
        "aegis-terra1" => Ok(DutKind::AegisTerra1),
        "river-rc1-nano" => Ok(DutKind::RiverRc1Nano),
        "river-rc1-micro" => Ok(DutKind::RiverRc1Micro),
        "river-rc1-small" => Ok(DutKind::RiverRc1Small),
        "river-rc1-medium" => Ok(DutKind::RiverRc1Medium),
        other => Err(eyre::eyre!("unknown target: {other}")),
    }
}
