use thiserror::Error;

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("transport not open")]
    NotOpen,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("unsupported capability `{0}` for this transport")]
    UnsupportedCapability(&'static str),
    #[error("openocd rpc error: {0}")]
    OpenOcd(String),
    #[cfg(feature = "openocd")]
    #[error("openocd response parse error: {0}")]
    OpenOcdParse(#[from] crate::openocd::parse::ParseError),
    #[error("timeout after {millis} ms")]
    Timeout { millis: u64 },
    #[error("idcode mismatch: got 0x{got:08x}, expected 0x{expected:08x}")]
    IdcodeMismatch { got: u32, expected: u32 },
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("device not found: {0}")]
    DeviceNotFound(String),
}
