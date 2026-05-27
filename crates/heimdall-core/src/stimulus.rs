use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stimulus {
    pub budget: StepBudget,
    /// Named input drives. Keys are usually pad names like `"io_0"`. Empty
    /// when the test doesn't drive anything (e.g. CPU bring-up via debug).
    #[serde(default)]
    pub inputs: BTreeMap<String, bool>,
}

impl Stimulus {
    pub fn new(budget: StepBudget) -> Self {
        Self {
            budget,
            inputs: BTreeMap::new(),
        }
    }

    pub fn with_input(mut self, key: impl Into<String>, value: bool) -> Self {
        self.inputs.insert(key.into(), value);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum StepBudget {
    Cycles { count: u64 },
    Duration { millis: u64 },
}

impl StepBudget {
    pub fn cycles(count: u64) -> Self {
        Self::Cycles { count }
    }

    pub fn duration(self) -> Option<Duration> {
        match self {
            Self::Duration { millis } => Some(Duration::from_millis(millis)),
            Self::Cycles { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_roundtrip() {
        let b = StepBudget::cycles(1_000);
        let j = serde_json::to_string(&b).unwrap();
        let back: StepBudget = serde_json::from_str(&j).unwrap();
        assert_eq!(back, b);
    }

    #[test]
    fn stimulus_inputs_default_empty() {
        let s = Stimulus::new(StepBudget::cycles(100));
        assert!(s.inputs.is_empty());
    }

    #[test]
    fn stimulus_with_input() {
        let s = Stimulus::new(StepBudget::cycles(10))
            .with_input("io_0", true)
            .with_input("io_1", false);
        assert_eq!(s.inputs.get("io_0"), Some(&true));
        assert_eq!(s.inputs.get("io_1"), Some(&false));
    }

    #[test]
    fn stimulus_inputs_roundtrip() {
        let s = Stimulus::new(StepBudget::cycles(50)).with_input("io_2", true);
        let j = serde_json::to_string(&s).unwrap();
        let back: Stimulus = serde_json::from_str(&j).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn stimulus_backward_compat_no_inputs_field() {
        // Old JSON without "inputs" should deserialize with empty map.
        let j = r#"{"budget":{"kind":"cycles","count":2048}}"#;
        let s: Stimulus = serde_json::from_str(j).unwrap();
        assert_eq!(s.budget, StepBudget::cycles(2048));
        assert!(s.inputs.is_empty());
    }
}
