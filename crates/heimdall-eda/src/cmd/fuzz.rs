use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

use clap::Args as ClapArgs;
use eyre::Result;
use heimdall::core::{DutId, DutKind, State, StepBudget, ValueRepr, Verdict};
use heimdall::driver::{Dut, MockDriver};
use heimdall::fuzzer::{
    BitFlipMutator, FuzzerEngine, IterCallback, RawAsmGen, RoundRobinScheduler, Rv64,
};
use heimdall::golden::MockGoldenModel;
use heimdall::test::Runner;
use indicatif::{ProgressBar, ProgressStyle};
use serde::Serialize;

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

    /// Submit the fuzz session as a daemon job against the named DUT
    /// instead of running locally with MockDriver. The daemon at
    /// `--daemon-url` must have the DUT in its registry and the
    /// `RiverFuzzFactory` enabled (built with `--features fuzzer,river`).
    #[arg(long)]
    pub remote_dut: Option<String>,

    /// Strict coverage mode: a sim-vs-silicon divergence flips a passing
    /// iteration's verdict to fail. Off by default. Only consulted for
    /// remote (daemon) submissions; the local path uses the engine
    /// default.
    #[arg(long, default_value_t = false)]
    pub strict_coverage: bool,

    /// Maximum wall-clock time to poll the daemon for the fuzz job to
    /// reach a terminal state. Defaults to enough headroom for slow sim
    /// rigs; bump for very long-running campaigns.
    #[arg(long, default_value_t = 3600)]
    pub poll_timeout_secs: u64,
}

pub async fn run(
    args: Args,
    _cfg_path: Option<std::path::PathBuf>,
    daemon_url: &str,
) -> Result<()> {
    if let Some(dut) = args.remote_dut.clone() {
        return run_remote(args, dut, daemon_url).await;
    }
    run_local(args).await
}

async fn run_local(args: Args) -> Result<()> {
    let target = parse_target(&args.target)?;
    tracing::info!(target = ?target, iterations = args.iterations, seed = args.seed, "fuzz start");

    let driver = MockDriver::new(target).with_state(State::new().with("x10", ValueRepr::U64(0x42)));
    let golden = MockGoldenModel::new(target);
    let mut dut = Dut::new(DutId::new("mock-dut"), target);

    let bar = ProgressBar::new(args.iterations).with_style(
        ProgressStyle::with_template(
            "{spinner:.cyan} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len} {msg} ({eta})",
        )
        .expect("valid template")
        .progress_chars("=>-"),
    );
    bar.enable_steady_tick(std::time::Duration::from_millis(120));

    let passes = Arc::new(AtomicU64::new(0));
    let fails = Arc::new(AtomicU64::new(0));
    let errors = Arc::new(AtomicU64::new(0));
    let skips = Arc::new(AtomicU64::new(0));
    let cb: IterCallback = {
        let bar = bar.clone();
        let passes = passes.clone();
        let fails = fails.clone();
        let errors = errors.clone();
        let skips = skips.clone();
        Arc::new(move |done, _total, verdict| {
            match verdict {
                Verdict::Pass => passes.fetch_add(1, Ordering::Relaxed),
                Verdict::Fail { .. } => fails.fetch_add(1, Ordering::Relaxed),
                Verdict::Skip { .. } => skips.fetch_add(1, Ordering::Relaxed),
                Verdict::Error { .. } => errors.fetch_add(1, Ordering::Relaxed),
            };
            bar.set_position(done);
            bar.set_message(format!(
                "pass={} fail={} err={}",
                passes.load(Ordering::Relaxed),
                fails.load(Ordering::Relaxed),
                errors.load(Ordering::Relaxed),
            ));
        })
    };

    let mut engine = FuzzerEngine::builder()
        .with_runner(Runner::builder().build())
        .with_generator(RawAsmGen::<Rv64>::new(args.insn_count))
        .with_mutator(BitFlipMutator)
        .with_scheduler(RoundRobinScheduler::new())
        .with_driver(driver)
        .with_golden(golden)
        .with_rng_seed(args.seed)
        .with_step_budget(StepBudget::cycles(args.cycles))
        .with_iter_callback(cb)
        .build();

    let report = engine.run(&mut dut, args.iterations).await?;
    bar.finish_with_message(format!(
        "pass={} fail={} err={}",
        report.passes, report.fails, report.errors
    ));
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

/// Submit the fuzz session as a `JobKind::Fuzz` to the daemon and poll
/// `/jobs/:id` until it reaches a terminal state. Streams a coarse-grained
/// progress bar based on the polled `state.state`; per-iteration progress
/// lives in the daemon's job-log stream which the web UI / TUI render.
async fn run_remote(args: Args, dut: String, daemon_url: &str) -> Result<()> {
    let body = NewFuzzJob {
        dut: dut.clone(),
        kind: FuzzKindPayload {
            kind: "fuzz",
            iterations: args.iterations,
            seed: args.seed,
            insn_count: args.insn_count,
            cycles: args.cycles,
            strict_coverage: args.strict_coverage,
        },
    };
    let client = reqwest::Client::new();
    let url = format!("{daemon_url}/jobs");
    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(eyre::Report::from)?;
    if !resp.status().is_success() {
        let status = resp.status();
        let detail = resp.text().await.unwrap_or_default();
        return Err(eyre::eyre!("daemon returned {status}: {detail}"));
    }
    let created: serde_json::Value = resp.json().await.map_err(eyre::Report::from)?;
    let job_id = created["id"]
        .as_str()
        .ok_or_else(|| eyre::eyre!("daemon response missing job id: {created}"))?
        .to_string();
    tracing::info!(job = %job_id, dut = %dut, "fuzz job submitted");

    let bar = ProgressBar::new_spinner().with_style(
        ProgressStyle::with_template("{spinner:.cyan} [{elapsed_precise}] {msg}")
            .expect("valid template"),
    );
    bar.enable_steady_tick(Duration::from_millis(250));
    bar.set_message(format!("submitted (job={})", short_id(&job_id)));

    let deadline = Instant::now() + Duration::from_secs(args.poll_timeout_secs);
    let job_url = format!("{daemon_url}/jobs/{job_id}");
    loop {
        if Instant::now() >= deadline {
            bar.finish_with_message("poll timed out");
            return Err(eyre::eyre!(
                "fuzz job {job_id} did not reach terminal state within {}s",
                args.poll_timeout_secs
            ));
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
        let resp = client
            .get(&job_url)
            .send()
            .await
            .map_err(eyre::Report::from)?;
        if !resp.status().is_success() {
            continue;
        }
        let body: serde_json::Value = resp.json().await.map_err(eyre::Report::from)?;
        let state = body["state"]["state"].as_str().unwrap_or("");
        bar.set_message(format!("state={state}"));
        match state {
            "done" => {
                let verdict = body["state"]["detail"]["kind"].as_str().unwrap_or("?");
                bar.finish_with_message(format!("done/{verdict}"));
                tracing::info!(job = %job_id, verdict, "fuzz job complete");
                if verdict != "pass" {
                    return Err(eyre::eyre!("fuzz verdict was {verdict}, not pass"));
                }
                return Ok(());
            }
            "failed" => {
                let detail = body["state"]["detail"].as_str().unwrap_or("(no detail)");
                bar.finish_with_message(format!("failed: {detail}"));
                return Err(eyre::eyre!("fuzz job failed: {detail}"));
            }
            "cancelled" => {
                bar.finish_with_message("cancelled");
                return Err(eyre::eyre!("fuzz job was cancelled"));
            }
            _ => continue,
        }
    }
}

#[derive(Serialize)]
struct NewFuzzJob {
    dut: String,
    kind: FuzzKindPayload,
}

#[derive(Serialize)]
struct FuzzKindPayload {
    kind: &'static str,
    iterations: u64,
    seed: u64,
    insn_count: usize,
    cycles: u64,
    strict_coverage: bool,
}

fn short_id(s: &str) -> &str {
    if s.len() > 8 { &s[..8] } else { s }
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
