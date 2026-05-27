//! FuzzerEngine main loop.
//!
//! Reuses `heimdall_test::Runner::run_one` per iteration so directed-test and
//! fuzz paths are bug-compatible.

use std::time::{Duration, Instant};

use heimdall_core::{SeedId, StepBudget, Verdict};
use heimdall_driver::{Dut, TestDriver};
use heimdall_golden::GoldenModel;
use heimdall_test::Runner;
use rand::RngCore;
use rand::SeedableRng;
use rand::rngs::StdRng;
use tracing::{instrument, warn};

use crate::corpus::{Corpus, CorpusEntry};
use crate::error::{FuzzerError, Result};
use crate::fuzz_test::AdHocFuzzTest;
use crate::traits::{Generator, Mutator, Scheduler, SchedulerChoice};

#[derive(Debug, Clone)]
pub struct DivergenceFinding {
    pub seed: SeedId,
    pub iteration: u64,
    pub sim_only_bits: usize,
    pub silicon_only_bits: usize,
    pub common_bits: usize,
}

#[derive(Debug, Clone)]
pub struct FuzzReport {
    pub iterations: u64,
    pub passes: u64,
    pub fails: u64,
    pub errors: u64,
    pub skips: u64,
    pub corpus_size: usize,
    pub coverage_bits: usize,
    pub silicon_coverage_bits: usize,
    pub divergences: Vec<DivergenceFinding>,
    pub elapsed: Duration,
}

pub struct FuzzerEngine<G, M, S, D, GM>
where
    G: Generator,
    M: Mutator,
    S: Scheduler,
    D: TestDriver,
    GM: GoldenModel,
{
    runner: Runner,
    generator: G,
    mutator: M,
    scheduler: S,
    driver: D,
    golden: GM,
    corpus: Corpus,
    rng: StdRng,
    step_budget: StepBudget,
    coverage_map: crate::coverage::CoverageMap,
    silicon_coverage_map: crate::coverage::CoverageMap,
    strict_coverage: bool,
    divergences: Vec<DivergenceFinding>,
}

pub struct FuzzerEngineBuilder<G, M, S, D, GM> {
    runner: Option<Runner>,
    generator: Option<G>,
    mutator: Option<M>,
    scheduler: Option<S>,
    driver: Option<D>,
    golden: Option<GM>,
    rng_seed: u64,
    step_budget: StepBudget,
    strict_coverage: bool,
}

impl<G, M, S, D, GM> Default for FuzzerEngineBuilder<G, M, S, D, GM>
where
    G: Generator,
    M: Mutator,
    S: Scheduler,
    D: TestDriver,
    GM: GoldenModel,
{
    fn default() -> Self {
        Self {
            runner: None,
            generator: None,
            mutator: None,
            scheduler: None,
            driver: None,
            golden: None,
            rng_seed: 0,
            step_budget: StepBudget::cycles(1000),
            strict_coverage: false,
        }
    }
}

impl<G, M, S, D, GM> FuzzerEngineBuilder<G, M, S, D, GM>
where
    G: Generator,
    M: Mutator,
    S: Scheduler,
    D: TestDriver,
    GM: GoldenModel,
{
    pub fn with_runner(mut self, runner: Runner) -> Self {
        self.runner = Some(runner);
        self
    }
    pub fn with_generator(mut self, generator: G) -> Self {
        self.generator = Some(generator);
        self
    }
    pub fn with_mutator(mut self, mutator: M) -> Self {
        self.mutator = Some(mutator);
        self
    }
    pub fn with_scheduler(mut self, scheduler: S) -> Self {
        self.scheduler = Some(scheduler);
        self
    }
    pub fn with_driver(mut self, driver: D) -> Self {
        self.driver = Some(driver);
        self
    }
    pub fn with_golden(mut self, golden: GM) -> Self {
        self.golden = Some(golden);
        self
    }
    pub fn with_rng_seed(mut self, seed: u64) -> Self {
        self.rng_seed = seed;
        self
    }
    pub fn with_step_budget(mut self, budget: StepBudget) -> Self {
        self.step_budget = budget;
        self
    }

    pub fn with_strict_coverage(mut self, strict: bool) -> Self {
        self.strict_coverage = strict;
        self
    }

    pub fn build(self) -> FuzzerEngine<G, M, S, D, GM> {
        FuzzerEngine {
            runner: self.runner.expect("runner not set"),
            generator: self.generator.expect("generator not set"),
            mutator: self.mutator.expect("mutator not set"),
            scheduler: self.scheduler.expect("scheduler not set"),
            driver: self.driver.expect("driver not set"),
            golden: self.golden.expect("golden not set"),
            corpus: Corpus::new(),
            rng: StdRng::seed_from_u64(self.rng_seed),
            step_budget: self.step_budget,
            coverage_map: crate::coverage::CoverageMap::default(),
            silicon_coverage_map: crate::coverage::CoverageMap::default(),
            strict_coverage: self.strict_coverage,
            divergences: Vec::new(),
        }
    }
}

impl<G, M, S, D, GM> FuzzerEngine<G, M, S, D, GM>
where
    G: Generator,
    M: Mutator,
    S: Scheduler,
    D: TestDriver,
    GM: GoldenModel,
{
    pub fn builder() -> FuzzerEngineBuilder<G, M, S, D, GM> {
        FuzzerEngineBuilder::default()
    }

    pub fn corpus(&self) -> &Corpus {
        &self.corpus
    }

    #[instrument(skip(self, dut), fields(dut = %dut.id))]
    pub async fn run(&mut self, dut: &mut Dut, iterations: u64) -> Result<FuzzReport> {
        let started = Instant::now();
        let mut passes = 0u64;
        let mut fails = 0u64;
        let mut errors = 0u64;
        let mut skips = 0u64;

        for iter in 0..iterations {
            self.scheduler.observe_corpus(&self.corpus.novel_indices());
            let choice = self.scheduler.next(self.corpus.len(), iter);
            let (seed_id, parent_seed) = match choice {
                SchedulerChoice::GenerateFresh => (SeedId(self.rng.next_u64()), None),
                SchedulerChoice::MutateAt(idx) => {
                    let parent = self.corpus.get_by_index(idx).ok_or(FuzzerError::NoSeeds)?;
                    (SeedId(self.rng.next_u64()), Some(parent.seed))
                }
            };

            let artifact = match choice {
                SchedulerChoice::GenerateFresh => self.generator.generate(&mut self.rng, seed_id),
                SchedulerChoice::MutateAt(idx) => {
                    let parent_artifact = self
                        .corpus
                        .get_by_index(idx)
                        .expect("parent existed above")
                        .artifact
                        .clone();
                    self.mutator.mutate(&parent_artifact, &mut self.rng)
                }
            };

            let test = AdHocFuzzTest::new(
                format!("fuzz-{seed_id}"),
                self.driver.target(),
                artifact.clone(),
                self.step_budget,
            );

            let res = self
                .runner
                .run_one(&test, dut, &mut self.driver, &mut self.golden)
                .await?;

            match &res.verdict {
                Verdict::Pass => passes += 1,
                Verdict::Fail { .. } => fails += 1,
                Verdict::Skip { .. } => skips += 1,
                Verdict::Error { .. } => errors += 1,
            }

            // Add to corpus if generated fresh or if mutated artifact differs from parent.
            // Dedup happens inside Corpus::add via sha.
            let entry = CorpusEntry {
                seed: seed_id,
                artifact,
                parent: parent_seed,
                last_verdict: None,
                last_snapshot: None,
                last_was_novel: false,
            };
            let stored_seed = self.corpus.add(entry);
            self.corpus.update_verdict(stored_seed, &res.verdict);

            // Capture both sides' coverage snapshots first.
            let sim_snapshot = self
                .golden
                .coverage()
                .map(|src| crate::coverage::CoverageSnapshot::from_bytes(src.snapshot()));
            let silicon_snapshot = self
                .driver
                .coverage()
                .map(|src| crate::coverage::CoverageSnapshot::from_bytes(src.snapshot()));

            // Merge into the global maps.
            let novel = match &sim_snapshot {
                Some(s) => self.coverage_map.merge(s),
                None => false,
            };
            if let Some(s) = &silicon_snapshot {
                self.silicon_coverage_map.merge(s);
            }
            if let Some(s) = sim_snapshot.clone() {
                self.corpus.update_coverage(stored_seed, s, novel);
            }

            // Compare for divergence (only meaningful when both sides have snapshots).
            let divergence = match (&sim_snapshot, &silicon_snapshot) {
                (Some(sim), Some(sil)) => Some(sim.diff_from(sil)),
                _ => None,
            };
            if let Some(d) = divergence.as_ref() {
                if d.is_divergent() {
                    let finding = DivergenceFinding {
                        seed: seed_id,
                        iteration: iter,
                        sim_only_bits: d.self_only_bits,
                        silicon_only_bits: d.other_only_bits,
                        common_bits: d.common_bits,
                    };
                    if self.strict_coverage && matches!(res.verdict, Verdict::Pass) {
                        passes -= 1;
                        fails += 1;
                    }
                    self.divergences.push(finding);
                }
            }

            if matches!(res.verdict, Verdict::Error { .. }) {
                warn!(?res.verdict, iteration = iter, "fuzz iteration errored");
            }
        }

        Ok(FuzzReport {
            iterations,
            passes,
            fails,
            errors,
            skips,
            corpus_size: self.corpus.len(),
            coverage_bits: self.coverage_map.bits_set(),
            silicon_coverage_bits: self.silicon_coverage_map.bits_set(),
            divergences: std::mem::take(&mut self.divergences),
            elapsed: started.elapsed(),
        })
    }
}
