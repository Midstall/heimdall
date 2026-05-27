use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("invalid id: {0}")]
    InvalidId(String),
    #[error("artifact too large: {actual} > {max}")]
    ArtifactTooLarge { actual: usize, max: usize },
    #[error("serde: {0}")]
    Serde(#[from] serde_json::Error),
}
