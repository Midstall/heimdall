use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

/// A typed architectural-state value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "kebab-case")]
pub enum ValueRepr {
    U64(u64),
    Bytes(Vec<u8>),
    Bool(bool),
}

impl fmt::Display for ValueRepr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::U64(v) => write!(f, "0x{:x}", v),
            Self::Bytes(b) => write!(f, "{}", hex::encode(b)),
            Self::Bool(b) => write!(f, "{}", b),
        }
    }
}

/// A snapshot of architectural state. Ordered map so diffs are deterministic.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct State {
    pub fields: BTreeMap<String, ValueRepr>,
    pub captured_after: Option<Duration>,
}

impl State {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, field: impl Into<String>, value: ValueRepr) -> Self {
        self.fields.insert(field.into(), value);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_deterministic_order() {
        let s = State::new()
            .with("b", ValueRepr::U64(2))
            .with("a", ValueRepr::U64(1));
        let keys: Vec<&String> = s.fields.keys().collect();
        assert_eq!(keys, vec![&"a".to_string(), &"b".to_string()]);
    }

    #[test]
    fn value_display() {
        assert_eq!(ValueRepr::U64(0x42).to_string(), "0x42");
        assert_eq!(ValueRepr::Bool(true).to_string(), "true");
    }
}
