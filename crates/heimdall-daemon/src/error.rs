use thiserror::Error;

#[derive(Debug, Error)]
pub enum DaemonError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[cfg(feature = "sqlite")]
    #[error("sqlx: {0}")]
    Sqlx(#[from] sqlx::Error),
    #[cfg(feature = "sqlite")]
    #[error("migrate: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("serde_json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("driver: {0}")]
    Driver(#[from] heimdall_driver::DriverError),
    #[error("golden: {0}")]
    Golden(#[from] heimdall_golden::GoldenError),
    #[error("test: {0}")]
    Test(#[from] heimdall_test::TestError),
    #[error("unknown job id {0}")]
    UnknownJob(String),
    #[error("unknown dut id {0}")]
    UnknownDut(String),
    #[error("lease expired or never held for dut {0}")]
    LeaseExpired(String),
    #[error("config: {0}")]
    Config(String),
    #[error("unsupported operation: {0}")]
    Unsupported(&'static str),
    #[error("dump format: {0}")]
    DumpFormat(String),
}

pub type Result<T> = std::result::Result<T, DaemonError>;
