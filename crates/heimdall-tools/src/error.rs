use thiserror::Error;

#[derive(Debug, Error)]
pub enum ToolError {
    #[error("no tool in chain accepts input kind {0:?}")]
    NoMatch(heimdall_core::ArtifactKind),
    #[error("tool `{tool}` exited with status {status}: {stderr}")]
    BadExit {
        tool: String,
        status: i32,
        stderr: String,
    },
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("tool `{0}` not found on PATH and no explicit path given")]
    NotFound(String),
    #[error("tool `{tool}` produced empty output")]
    EmptyOutput { tool: String },
}
