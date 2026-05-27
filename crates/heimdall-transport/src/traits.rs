use async_trait::async_trait;
use std::fmt;

use crate::error::TransportError;

pub type Result<T> = std::result::Result<T, TransportError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportKind {
    Jtag,
    Serial,
    Usb,
    Gpio,
    Psu,
    LogicAnalyzer,
    Mock,
}

impl fmt::Display for TransportKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Jtag => f.write_str("jtag"),
            Self::Serial => f.write_str("serial"),
            Self::Usb => f.write_str("usb"),
            Self::Gpio => f.write_str("gpio"),
            Self::Psu => f.write_str("psu"),
            Self::LogicAnalyzer => f.write_str("logic-analyzer"),
            Self::Mock => f.write_str("mock"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetTarget {
    System,
    Cpu,
    DebugModule,
}

#[async_trait]
pub trait Transport: Send + Sync {
    fn kind(&self) -> TransportKind;
    async fn open(&mut self) -> Result<()>;
    async fn close(&mut self) -> Result<()>;
    async fn reset(&mut self, target: ResetTarget) -> Result<()>;
}
