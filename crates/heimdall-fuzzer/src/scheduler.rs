use crate::traits::{Scheduler, SchedulerChoice};

/// Round-robin: mutate corpus[i % len] for each iteration, or generate
/// fresh when the corpus is empty.
#[derive(Debug, Default)]
pub struct RoundRobinScheduler;

impl RoundRobinScheduler {
    pub fn new() -> Self {
        Self
    }
}

impl Scheduler for RoundRobinScheduler {
    fn next(&mut self, corpus_size: usize, iteration: u64) -> SchedulerChoice {
        if corpus_size == 0 {
            SchedulerChoice::GenerateFresh
        } else {
            SchedulerChoice::MutateAt((iteration as usize) % corpus_size)
        }
    }
}

/// Coverage-guided power scheduler. Prioritizes corpus entries that produced
/// new coverage bits in their last iteration. Falls back to RoundRobinScheduler
/// when no novel entries are known.
#[derive(Debug, Default)]
pub struct PowerScheduler {
    novel_indices: Vec<usize>,
    next_within_novel: usize,
    fallback: RoundRobinScheduler,
}

impl PowerScheduler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update_novelty(&mut self, novel_indices: Vec<usize>) {
        self.novel_indices = novel_indices;
        if self.next_within_novel >= self.novel_indices.len() {
            self.next_within_novel = 0;
        }
    }
}

impl Scheduler for PowerScheduler {
    fn next(&mut self, corpus_size: usize, iteration: u64) -> SchedulerChoice {
        if !self.novel_indices.is_empty() {
            let idx = self.novel_indices[self.next_within_novel % self.novel_indices.len()];
            self.next_within_novel += 1;
            return SchedulerChoice::MutateAt(idx);
        }
        self.fallback.next(corpus_size, iteration)
    }

    fn observe_corpus(&mut self, novel_indices: &[usize]) {
        self.update_novelty(novel_indices.to_vec());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_corpus_forces_generate() {
        let mut s = RoundRobinScheduler::new();
        assert_eq!(s.next(0, 0), SchedulerChoice::GenerateFresh);
    }

    #[test]
    fn nonempty_corpus_wraps() {
        let mut s = RoundRobinScheduler::new();
        assert_eq!(s.next(3, 0), SchedulerChoice::MutateAt(0));
        assert_eq!(s.next(3, 1), SchedulerChoice::MutateAt(1));
        assert_eq!(s.next(3, 2), SchedulerChoice::MutateAt(2));
        assert_eq!(s.next(3, 3), SchedulerChoice::MutateAt(0));
    }
}

#[cfg(test)]
mod power_tests {
    use super::*;

    #[test]
    fn power_uses_novel_indices_when_available() {
        let mut s = PowerScheduler::new();
        s.update_novelty(vec![3, 5, 7]);
        assert_eq!(s.next(10, 0), SchedulerChoice::MutateAt(3));
        assert_eq!(s.next(10, 1), SchedulerChoice::MutateAt(5));
        assert_eq!(s.next(10, 2), SchedulerChoice::MutateAt(7));
        assert_eq!(s.next(10, 3), SchedulerChoice::MutateAt(3));
    }

    #[test]
    fn power_falls_back_to_round_robin_when_no_novelty() {
        let mut s = PowerScheduler::new();
        // No novel indices set.
        assert_eq!(s.next(3, 0), SchedulerChoice::MutateAt(0));
        assert_eq!(s.next(3, 1), SchedulerChoice::MutateAt(1));
    }

    #[test]
    fn power_falls_back_when_novelty_cleared() {
        let mut s = PowerScheduler::new();
        s.update_novelty(vec![2]);
        assert_eq!(s.next(3, 0), SchedulerChoice::MutateAt(2));
        s.update_novelty(vec![]);
        assert_eq!(s.next(3, 1), SchedulerChoice::MutateAt(1));
    }
}
