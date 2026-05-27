use thiserror::Error;

#[derive(Debug, Error)]
pub enum GoldenError {
    #[error("backend not loaded; call `load` first")]
    NotLoaded,
    #[error("subprocess error: {0}")]
    Io(#[from] std::io::Error),
    #[error("spike exited with status {status}: {stderr}")]
    SpikeBadExit { status: i32, stderr: String },
    #[error("could not parse spike output: {0}")]
    ParseSpike(String),
    #[error("unsupported budget {0:?} for backend `{1}`")]
    UnsupportedBudget(heimdall_core::StepBudget, &'static str),
    #[error("invalid netlist: {0}")]
    NetlistParse(String),
}
