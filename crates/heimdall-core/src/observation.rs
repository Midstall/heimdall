use crate::state::State;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Observation {
    pub state: State,
    pub elapsed: Duration,
}

impl Observation {
    pub fn new(state: State, elapsed: Duration) -> Self {
        Self { state, elapsed }
    }
}
