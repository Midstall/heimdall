use thiserror::Error;

#[derive(Debug, Error)]
pub enum DriverError {
    #[error("transport: {0}")]
    Transport(#[from] heimdall_transport::TransportError),
    #[error("tool: {0}")]
    Tool(#[from] heimdall_tools::ToolError),
    #[error("golden: {0}")]
    Golden(#[from] heimdall_golden::GoldenError),
    #[error("required transport kind `{0}` not provided")]
    MissingTransport(heimdall_transport::TransportKind),
    #[error("dut returned unexpected idcode 0x{got:08x}, expected 0x{expected:08x}")]
    IdcodeMismatch { got: u32, expected: u32 },
    #[error("driver state error: {0}")]
    State(&'static str),
}
