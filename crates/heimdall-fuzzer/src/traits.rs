use heimdall_core::{Artifact, DutKind, SeedId};
use rand::RngCore;

/// Produces an Artifact from a SeedId and an RNG. Deterministic given the
/// same seed value and RNG state.
pub trait Generator: Send + Sync {
    fn target(&self) -> DutKind;
    fn name(&self) -> &str;
    fn generate(&mut self, rng: &mut dyn RngCore, seed: SeedId) -> Artifact;
}

/// Blanket impl so callers that need a runtime choice between
/// generators (e.g. the daemon worker switching on
/// `JobKind::Fuzz.generator`) can hand a `Box<dyn Generator>` to the
/// engine builder without duplicating the build pipeline per concrete
/// type. The trait stays object-safe because every method takes
/// `&mut self` or `&self` and returns owned values.
impl<G: Generator + ?Sized> Generator for Box<G> {
    fn target(&self) -> DutKind {
        (**self).target()
    }
    fn name(&self) -> &str {
        (**self).name()
    }
    fn generate(&mut self, rng: &mut dyn RngCore, seed: SeedId) -> Artifact {
        (**self).generate(rng, seed)
    }
}

/// Mutates a parent Artifact. Returns a new Artifact with the same kind.
pub trait Mutator: Send + Sync {
    fn name(&self) -> &str;
    fn mutate(&mut self, parent: &Artifact, rng: &mut dyn RngCore) -> Artifact;
}

/// Picks the next action: generate fresh, or mutate an existing corpus entry.
pub trait Scheduler: Send + Sync {
    fn next(&mut self, corpus_size: usize, iteration: u64) -> SchedulerChoice;
    /// Called before each iteration's `next` with the current set of novel
    /// corpus indices. Default implementation is a no-op; PowerScheduler
    /// overrides this to update its priority list.
    fn observe_corpus(&mut self, _novel_indices: &[usize]) {}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerChoice {
    GenerateFresh,
    MutateAt(usize),
}
