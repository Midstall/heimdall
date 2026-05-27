use thiserror::Error;

#[derive(Debug, Error)]
pub enum TuiError {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("ws: {0}")]
    Ws(#[from] tokio_tungstenite::tungstenite::Error),
    #[error("ws closed unexpectedly")]
    WsClosed,
    #[error("serde_json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid daemon url: {0}")]
    BadUrl(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, TuiError>;
