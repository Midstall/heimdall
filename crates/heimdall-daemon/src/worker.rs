//! Worker: pulls JobIds from the queue receiver, dispatches each through the
//! DriverRegistry, runs the resulting (driver, golden, test) trio via the
//! existing Runner, and transitions state via JobQueue.

use std::sync::Arc;
use std::time::Duration;

use heimdall_driver::Dut;
use heimdall_test::Runner;
use tracing::{error, info, instrument};

use crate::dut_registry::DutRegistry;
use crate::error::Result;
use crate::factory::DriverRegistry;
use crate::lease::{LeaseManager, LeaseTtl};
use crate::queue::{JobQueue, JobQueueReceiver};
use crate::types::{Event, JobId, JobState, VerdictSummary};

pub struct Worker {
    queue: JobQueue,
    leases: LeaseManager,
    registry: DriverRegistry,
    dut_registry: Arc<DutRegistry>,
    loaded_programs: crate::loaded_programs::LoadedProgramCache,
    isa_probes: crate::isa_probes::IsaProbeCache,
    blobs: Option<Arc<dyn crate::store::BlobStore>>,
    cancellations: crate::cancellations::CancellationRegistry,
}

impl Worker {
    pub fn new(queue: JobQueue, leases: LeaseManager) -> Self {
        Self::new_with_registry(queue, leases, DriverRegistry::default_mock())
    }

    pub fn new_with_registry(
        queue: JobQueue,
        leases: LeaseManager,
        registry: DriverRegistry,
    ) -> Self {
        Self {
            queue,
            leases,
            registry,
            dut_registry: Arc::new(DutRegistry::new()),
            loaded_programs: crate::loaded_programs::LoadedProgramCache::new(),
            isa_probes: crate::isa_probes::IsaProbeCache::new(),
            blobs: None,
            cancellations: crate::cancellations::CancellationRegistry::new(),
        }
    }

    pub fn with_dut_registry(mut self, dut_registry: Arc<DutRegistry>) -> Self {
        self.dut_registry = dut_registry;
        self
    }

    /// Inject the daemon-wide loaded-program cache so per-iter fuzz
    /// artifacts surface in `/jobs/:id/disasm`. Same handle the
    /// AppState carries.
    pub fn with_loaded_program_cache(
        mut self,
        cache: crate::loaded_programs::LoadedProgramCache,
    ) -> Self {
        self.loaded_programs = cache;
        self
    }

    /// Inject the daemon-wide ISA probe cache. Same handle AppState
    /// carries; the worker's fuzz dispatch lazily probes a DUT's
    /// `misa` on first contact and stores the result here so
    /// subsequent fuzz jobs against the same DUT skip the JTAG
    /// roundtrip.
    pub fn with_isa_probe_cache(mut self, cache: crate::isa_probes::IsaProbeCache) -> Self {
        self.isa_probes = cache;
        self
    }

    /// Inject the BlobStore the worker writes fuzz iter programs to.
    /// Without it set, fuzz dispatches still work, but the disasm
    /// panel won't survive a daemon restart (in-memory cache only).
    /// The runtime constructor wires this from AppState's BlobStore.
    pub fn with_blob_store(mut self, blobs: Arc<dyn crate::store::BlobStore>) -> Self {
        self.blobs = Some(blobs);
        self
    }

    /// Inject the cancellation registry. The worker registers a
    /// token per in-flight job; the cancel HTTP route on the same
    /// registry fires it to abort `engine.run()` / `run_one()`
    /// from the UI.
    pub fn with_cancellation_registry(
        mut self,
        cancellations: crate::cancellations::CancellationRegistry,
    ) -> Self {
        self.cancellations = cancellations;
        self
    }

    #[instrument(skip(self, recv))]
    pub async fn run(self, mut recv: JobQueueReceiver) {
        while let Some(job_id) = recv.rx.recv().await {
            if let Err(e) = self.handle_one(job_id).await {
                error!(error = %e, "job dispatch failed");
            }
        }
    }
}

impl Worker {
    #[instrument(skip(self))]
    async fn handle_one(&self, job_id: JobId) -> Result<()> {
        let store = self.queue.store();
        let job = match store.get_job(job_id).await? {
            Some(j) => j,
            None => return Ok(()),
        };

        self.queue.transition(job_id, JobState::Running).await?;
        let lease_ttl = self
            .dut_registry
            .lookup(&job.dut)
            .map(|r| LeaseTtl(Duration::from_secs(r.timeouts.lease_secs)))
            .unwrap_or_default();
        let lease = self
            .leases
            .acquire_with_ttl(job.dut.clone(), job_id, lease_ttl)
            .await?;
        self.queue
            .bus()
            .publish(Event::LeaseAcquired {
                lease: lease.id,
                dut: job.dut.clone(),
                holder: job_id,
            })
            .await?;

        let logger = crate::job_logger::JobLogger::new(self.queue.bus().clone(), job_id);
        logger
            .log_keyed(
                crate::types::LogLevel::Info,
                "log.worker.dispatching",
                &[("summary", job.kind.summary())],
            )
            .await;

        let cancel_token = self.cancellations.register(job_id);
        let outcome = tokio::select! {
            res = self.run_via_registry(&job, job_id, logger.clone()) => res,
            _ = cancel_token.cancelled() => Err(crate::error::DaemonError::Cancelled),
        };
        self.cancellations.remove(job_id);
        let next_state = match outcome {
            Ok(verdict) => JobState::Done(VerdictSummary::from(&verdict)),
            Err(crate::error::DaemonError::Cancelled) => {
                logger
                    .log_keyed(crate::types::LogLevel::Warn, "log.worker.cancelled", &[])
                    .await;
                JobState::Cancelled
            }
            Err(err) => {
                let msg = err.to_string();
                logger.log(crate::types::LogLevel::Error, msg.clone()).await;
                JobState::Failed(msg)
            }
        };

        self.queue.transition(job_id, next_state).await?;
        // Intentionally keep the loaded-program entry past job
        // termination so the post-mortem disasm panel still shows the
        // last-iter program. The cache lives only in this daemon
        // process and is bounded by jobs-since-restart; an LRU cap is
        // a follow-up if growth ever becomes a concern.
        self.leases.release(&job.dut, lease.id).await?;
        self.queue
            .bus()
            .publish(Event::LeaseReleased {
                lease: lease.id,
                dut: job.dut.clone(),
            })
            .await?;
        info!(job = %job_id, "completed");
        Ok(())
    }

    async fn run_via_registry(
        &self,
        job: &crate::types::Job,
        job_id: JobId,
        logger: crate::job_logger::JobLogger,
    ) -> std::result::Result<heimdall_core::Verdict, crate::error::DaemonError> {
        let dispatch = self.registry.dispatch(job).await?;
        match dispatch {
            crate::factory::Dispatch::Test(bundle) => run_test_dispatch(job, bundle, logger).await,
            crate::factory::Dispatch::Fuzz(fuzz) => {
                self.run_fuzz_dispatch(job, job_id, fuzz, logger).await
            }
        }
    }
}

async fn run_test_dispatch(
    job: &crate::types::Job,
    bundle: crate::factory::DispatchBundle,
    logger: crate::job_logger::JobLogger,
) -> std::result::Result<heimdall_core::Verdict, crate::error::DaemonError> {
    let runner = Runner::builder()
        .with_observer(Arc::new(logger) as Arc<dyn heimdall_test::StageObserver>)
        .build();
    let mut driver = bundle.driver;
    let mut golden = bundle.golden;
    let test = bundle.test;
    let mut dut = Dut::new(job.dut.clone(), job.dut_kind);
    let res = runner
        .run_one(&*test, &mut dut, &mut *driver, &mut *golden)
        .await?;
    Ok(res.verdict)
}

/// Drive a fuzz dispatch to completion. Builds a `FuzzerEngine` from the
/// boxed driver + golden the factory returned (the `Box<dyn _>` blanket
/// impls in heimdall-driver / heimdall-golden satisfy the engine's trait
/// bounds), wires the `JobLogger` as the per-iteration stage observer,
/// runs the configured number of iterations, and rolls the aggregate
/// report into a single `Verdict` for the job state.
#[cfg(feature = "fuzzer")]
impl Worker {
    async fn run_fuzz_dispatch(
        &self,
        job: &crate::types::Job,
        job_id: JobId,
        fuzz: crate::factory::FuzzDispatch,
        logger: crate::job_logger::JobLogger,
    ) -> std::result::Result<heimdall_core::Verdict, crate::error::DaemonError> {
        let loaded_programs = &self.loaded_programs;
        let isa_probes = &self.isa_probes;
        let store = self.queue.store();
        let blobs = self.blobs.as_ref();
        use heimdall_core::StepBudget;
        use heimdall_driver::Dut as DriverDut;
        use heimdall_fuzzer::{
            BitFlipMutator, FuzzerEngine, Generator, RawAsmGen, RoundRobinScheduler, Rv32, Rv64,
        };

        let cfg = fuzz.config;
        let mut driver = fuzz.driver;
        let golden = fuzz.golden;

        // Resolve the DUT's ISA. Order of preference:
        // 1. The [dut.isa] block from the registry (operator-authoritative).
        // 2. JTAG-probed `misa` result (cached after first dispatch).
        // 3. Library default RV64I+M for legacy configs.
        //
        // The probe only runs when the operator hasn't supplied a TOML
        // override AND the cache is cold. Lazy + cached keeps repeat
        // fuzz jobs cheap.
        let probed = if cfg.isa.is_none() {
            match isa_probes.get(&job.dut) {
                Some(p) => Some(p),
                None => {
                    // Cold cache: prepare the driver enough to read CSR
                    // misa, run the probe, store the result. We do this
                    // out-of-band before the engine starts; River's
                    // prepare() is idempotent so the engine's per-iter
                    // prepare runs again without issue.
                    let mut probe_dut = DriverDut::new(job.dut.clone(), job.dut_kind);
                    match driver.prepare(&mut probe_dut).await {
                        Ok(()) => match driver.probe_isa().await {
                            Ok(Some(probe)) => {
                                // `From<IsaProbe> for ProbedIsa` keeps
                                // this conversion declarative; the cache
                                // value and the local `Some(...)` use
                                // the same handle.
                                let entry: crate::isa_probes::ProbedIsa = probe.into();
                                isa_probes.insert(job.dut.clone(), entry.clone());
                                Some(entry)
                            }
                            Ok(None) => None,
                            Err(e) => {
                                // Probe failure isn't fatal: the worker
                                // falls back to the next precedence
                                // layer (default RV64+IM today, RV32I
                                // once the default-resolution change
                                // lands). Surface a log entry so an
                                // operator can see why the probe didn't
                                // populate the cache.
                                logger
                                    .log(
                                        crate::types::LogLevel::Warn,
                                        format!("misa probe failed: {e}; falling back to defaults"),
                                    )
                                    .await;
                                None
                            }
                        },
                        Err(e) => {
                            logger
                                .log(
                                    crate::types::LogLevel::Warn,
                                    format!(
                                        "isa probe prepare failed: {e}; falling back to defaults"
                                    ),
                                )
                                .await;
                            None
                        }
                    }
                }
            }
        } else {
            None
        };

        // resolve_isa prefers (1) the TOML spec, (2) the probe result,
        // (3) the library default. The helper handles all three cases
        // through a uniform IsaSpec view.
        // `From<&ProbedIsa> for IsaSpec` is the registry shape; lets
        // the precedence chain (TOML over probe over default) stay in
        // a single Option<&IsaSpec> form.
        let probe_as_spec = probed.as_ref().map(crate::dut_registry::IsaSpec::from);
        let effective_spec = cfg.isa.as_ref().or(probe_as_spec.as_ref());
        let (xlen, ext_set) = resolve_isa(effective_spec);

        // Choose the generator. `Box<dyn Generator>` is fed to the engine
        // builder via the blanket impl in heimdall_fuzzer::traits, so this
        // is the single point where the dispatch picks a concrete kind.
        // Cranelift requires the daemon to have been built with its
        // feature; surface a clean config error if not.
        let generator: Box<dyn Generator> = match cfg.generator {
            crate::types::GeneratorKind::RawAsm => match xlen {
                32 => Box::new(
                    RawAsmGen::<Rv32>::new(cfg.insn_count).with_extensions(ext_set.iter().copied()),
                ),
                _ => Box::new(
                    RawAsmGen::<Rv64>::new(cfg.insn_count).with_extensions(ext_set.iter().copied()),
                ),
            },
            #[cfg(feature = "cranelift")]
            crate::types::GeneratorKind::Cranelift => {
                let g =
                    heimdall_fuzzer::CraneliftGen::rv64()?.with_ops_per_function(cfg.insn_count);
                Box::new(g)
            }
            #[cfg(not(feature = "cranelift"))]
            crate::types::GeneratorKind::Cranelift => {
                return Err(crate::error::DaemonError::CraneliftFeatureDisabled);
            }
        };
        // Fuzz toolchain: wrap the generator's raw RV64 machine code in a
        // minimal ELF so the River driver's `load` can place it at p_paddr
        // and `run` can seed `dpc = entry`. Without this step the chain has
        // no tool that accepts `ArtifactKind::RawBytes` and compile errors
        // out before the first iteration ever reaches the DUT.
        let tools = heimdall_tools::ToolChain::new().with(std::sync::Arc::new(
            heimdall_tools::RawBytesToElfRiscv::new(),
        ));
        let runner = Runner::builder()
            .with_tools(tools)
            .with_observer(Arc::new(logger.clone()) as Arc<dyn heimdall_test::StageObserver>)
            .build();
        // Stash each iter's program bytes into the daemon-wide cache so the
        // `/jobs/:id/disasm` route can serve "what is this fuzz job running
        // right now" without re-deriving from the seed. The observer is
        // called BEFORE the runner consumes the artifact, so the cache is
        // populated for the entire run-and-observe window of every iter.
        let cache_handle = loaded_programs.clone();
        let job_id_capture = job_id;
        let program_observer: heimdall_fuzzer::ProgramCallback =
            std::sync::Arc::new(move |iter, artifact: &heimdall_core::Artifact| {
                cache_handle.record(
                    job_id_capture,
                    crate::loaded_programs::LoadedProgram {
                        iter,
                        kind: artifact.kind.clone(),
                        bytes: artifact.bytes.to_vec(),
                    },
                );
            });
        let mut engine = FuzzerEngine::builder()
            .with_runner(runner)
            .with_generator(generator)
            .with_mutator(BitFlipMutator)
            .with_scheduler(RoundRobinScheduler::new())
            .with_driver(driver)
            .with_golden(golden)
            .with_rng_seed(cfg.seed)
            .with_step_budget(StepBudget::cycles(cfg.cycles))
            .with_strict_coverage(cfg.strict_coverage)
            .with_program_observer(program_observer)
            .build();
        let mut dut = Dut::new(job.dut.clone(), job.dut_kind);
        let report_result = engine.run(&mut dut, cfg.iterations).await;
        // Persist whatever the cache holds before propagating errors. We
        // want the disasm panel to keep working after a daemon restart
        // even when the engine bailed mid-run (driver flake, signal,
        // etc.). Best-effort. Log on failure but don't surface, since
        // the primary verdict path is what matters here.
        if let Some(blobs) = blobs {
            persist_latest_program(loaded_programs, blobs.as_ref(), &*store, job_id, &logger).await;
        }
        let report = report_result?;

        logger
            .log(
                crate::types::LogLevel::Info,
                format!(
                    "fuzz complete: iterations={}, pass={}, fail={}, err={}, corpus={}, cov={}/{}",
                    report.iterations,
                    report.passes,
                    report.fails,
                    report.errors,
                    report.corpus_size,
                    report.coverage_bits,
                    report.silicon_coverage_bits,
                ),
            )
            .await;

        // Aggregate verdict: any fail or infrastructure error fails the job.
        // Coverage divergences with strict_coverage already flipped Pass to Fail
        // inside the engine, so they count toward `fails` here.
        if report.errors > 0 {
            Ok(heimdall_core::Verdict::Error {
                message: format!("fuzz reported {} infra errors", report.errors),
            })
        } else if report.fails > 0 {
            Ok(heimdall_core::Verdict::Fail {
                kind: heimdall_core::FailureKind::DiffMismatch {
                    field: "fuzz".into(),
                    got: heimdall_core::ValueRepr::U64(report.fails),
                    expected: heimdall_core::ValueRepr::U64(0),
                },
                evidence: vec![heimdall_core::Evidence {
                    label: "fuzz-fails".into(),
                    detail: format!(
                        "{} of {} iterations failed",
                        report.fails, report.iterations
                    ),
                }],
            })
        } else {
            Ok(heimdall_core::Verdict::Pass)
        }
    }
}

/// Fallback for fuzz dispatch when the daemon was built without the
/// `fuzzer` feature. Reports an explicit configuration error so the
/// failure mode is loud rather than silently routing to the test path.
#[cfg(not(feature = "fuzzer"))]
impl Worker {
    async fn run_fuzz_dispatch(
        &self,
        _job: &crate::types::Job,
        _job_id: JobId,
        _fuzz: crate::factory::FuzzDispatch,
        _logger: crate::job_logger::JobLogger,
    ) -> std::result::Result<heimdall_core::Verdict, crate::error::DaemonError> {
        Err(crate::error::DaemonError::FuzzerFeatureDisabled)
    }
}

/// Write the latest cached program for `job` into the BlobStore and
/// record the mapping in the JobStore. Best-effort: failures are
/// logged but never propagated, since persistence is a
/// disaster-recovery aid (the disasm panel after a daemon restart),
/// not part of the primary verdict flow.
#[cfg(feature = "fuzzer")]
async fn persist_latest_program(
    cache: &crate::loaded_programs::LoadedProgramCache,
    blobs: &dyn crate::store::BlobStore,
    store: &dyn crate::store::JobStore,
    job: JobId,
    logger: &crate::job_logger::JobLogger,
) {
    let Some(program) = cache.latest(job) else {
        return;
    };
    let blob_id = match blobs.put(&program.bytes).await {
        Ok(id) => id,
        Err(e) => {
            logger
                .log(
                    crate::types::LogLevel::Warn,
                    format!("persist fuzz program: blob put failed: {e}"),
                )
                .await;
            return;
        }
    };
    let r = store
        .set_job_program(
            job,
            crate::store::JobProgramRef {
                blob_id,
                kind: program.kind,
                iter: Some(program.iter),
            },
        )
        .await;
    if let Err(e) = r {
        logger
            .log(
                crate::types::LogLevel::Warn,
                format!("persist fuzz program: set_job_program failed: {e}"),
            )
            .await;
    }
}

/// Convenience type alias matching the worker's JobStore expectation.
pub type SharedJobStore = std::sync::Arc<dyn crate::store::JobStore>;

/// Resolve the effective `(xlen, extensions)` for a fuzz session.
///
/// Defers all the actual parsing to the `From<&IsaSpec>` impl that
/// lives on [`heimdall_fuzzer::ParsedIsa`]: the registry-side
/// `IsaSpec` carries strings, the typed `ParsedIsa` carries
/// `RvExtension` values, and the conversion silently drops
/// extension strings that aren't known to this build (forward-compat
/// for future ISA strings showing up in heimdall.toml).
///
/// When no spec is supplied: default to RV64 + `{I, M}` so existing
/// pre-isa heimdall.toml files keep working unchanged.
#[cfg(feature = "fuzzer")]
fn resolve_isa(
    spec: Option<&crate::dut_registry::IsaSpec>,
) -> (u8, Vec<heimdall_fuzzer::RvExtension>) {
    use heimdall_fuzzer::RvExtension;
    let Some(spec) = spec else {
        return (64, vec![RvExtension::I, RvExtension::M]);
    };
    let parsed: heimdall_fuzzer::ParsedIsa = spec.into();
    (parsed.xlen, parsed.extensions.into_iter().collect())
}

#[cfg(all(test, feature = "fuzzer"))]
mod tests {
    use super::*;
    use crate::dut_registry::IsaSpec;
    use heimdall_fuzzer::RvExtension;

    #[test]
    fn resolve_isa_default_when_unset() {
        let (xlen, exts) = resolve_isa(None);
        assert_eq!(xlen, 64);
        assert!(exts.contains(&RvExtension::I));
        assert!(exts.contains(&RvExtension::M));
    }

    #[test]
    fn resolve_isa_rv32i_minimal() {
        let spec = IsaSpec {
            xlen: 32,
            extensions: vec!["i".into()],
        };
        let (xlen, exts) = resolve_isa(Some(&spec));
        assert_eq!(xlen, 32);
        // I is implicit; the helper always includes it.
        assert!(exts.contains(&RvExtension::I));
        assert!(!exts.contains(&RvExtension::M));
    }

    #[test]
    fn resolve_isa_rv64imac_zicsr_full() {
        let spec = IsaSpec {
            xlen: 64,
            extensions: vec![
                "i".into(),
                "m".into(),
                "a".into(),
                "c".into(),
                "zicsr".into(),
            ],
        };
        let (xlen, exts) = resolve_isa(Some(&spec));
        assert_eq!(xlen, 64);
        for ext in [
            RvExtension::I,
            RvExtension::M,
            RvExtension::A,
            RvExtension::C,
            RvExtension::Zicsr,
        ] {
            assert!(exts.contains(&ext), "expected {ext:?} in resolved set");
        }
    }

    #[test]
    fn resolve_isa_unknown_extension_silently_dropped() {
        let spec = IsaSpec {
            xlen: 64,
            extensions: vec!["i".into(), "futureXYZ".into()],
        };
        let (_, exts) = resolve_isa(Some(&spec));
        // Unknown entries don't crash the worker; the resolved set
        // just doesn't include them. I is always present.
        assert!(exts.contains(&RvExtension::I));
    }
}
