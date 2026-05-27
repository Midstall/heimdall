use thiserror::Error;

#[derive(Debug, Error)]
pub enum FuzzerError {
    #[error("driver: {0}")]
    Driver(#[from] heimdall_driver::DriverError),
    #[error("golden: {0}")]
    Golden(#[from] heimdall_golden::GoldenError),
    #[error("test: {0}")]
    Test(#[from] heimdall_test::TestError),
    #[error("corpus empty and generator exhausted")]
    NoSeeds,
    #[error("generator: {0}")]
    Generator(String),
}

pub type Result<T> = std::result::Result<T, FuzzerError>;
