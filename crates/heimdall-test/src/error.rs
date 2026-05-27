use thiserror::Error;

#[derive(Debug, Error)]
pub enum TestError {
    #[error("driver: {0}")]
    Driver(#[from] heimdall_driver::DriverError),
    #[error("golden: {0}")]
    Golden(#[from] heimdall_golden::GoldenError),
    #[error("test build failed: {0}")]
    Build(String),
}
