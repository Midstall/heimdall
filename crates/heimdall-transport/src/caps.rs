use async_trait::async_trait;
use std::time::Duration;

use crate::traits::Result;

/// Unified trait combining `Transport` lifecycle methods and `GpioOps`
/// sync I/O. Provides a single trait-object surface so a driver can hold
/// `Box<dyn GpioTransport>` and still call open/close in addition to set/read.
///
/// Implemented automatically for any concrete type that implements both
/// `Transport` and `GpioOps`.
#[async_trait]
pub trait GpioTransport: Send + Sync {
    async fn open(&mut self) -> Result<()>;
    async fn close(&mut self) -> Result<()>;
    async fn reset(&mut self, target: crate::traits::ResetTarget) -> Result<()>;
    fn set(&mut self, line: u32, high: bool) -> Result<()>;
    fn read(&mut self, line: u32) -> Result<bool>;
    fn pulse(&mut self, line: u32, duration: Duration) -> Result<()>;
}

#[async_trait]
impl<T> GpioTransport for T
where
    T: crate::traits::Transport + GpioOps + Send + Sync,
{
    async fn open(&mut self) -> Result<()> {
        <T as crate::traits::Transport>::open(self).await
    }
    async fn close(&mut self) -> Result<()> {
        <T as crate::traits::Transport>::close(self).await
    }
    async fn reset(&mut self, target: crate::traits::ResetTarget) -> Result<()> {
        <T as crate::traits::Transport>::reset(self, target).await
    }
    fn set(&mut self, line: u32, high: bool) -> Result<()> {
        <T as GpioOps>::set(self, line, high)
    }
    fn read(&mut self, line: u32) -> Result<bool> {
        <T as GpioOps>::read(self, line)
    }
    fn pulse(&mut self, line: u32, duration: Duration) -> Result<()> {
        <T as GpioOps>::pulse(self, line, duration)
    }
}

#[async_trait]
pub trait JtagOps: Send + Sync {
    async fn scan_idcode(&mut self) -> Result<Vec<u32>>;
    async fn shift_dr(&mut self, ir: u32, bits: usize, data: &[u8]) -> Result<Vec<u8>>;
}

#[async_trait]
pub trait SerialOps: Send + Sync {
    async fn write_all(&mut self, bytes: &[u8]) -> Result<()>;
    async fn read_until(&mut self, delim: u8, timeout: Duration) -> Result<Vec<u8>>;
}

#[async_trait]
pub trait UsbOps: Send + Sync {
    async fn bulk_in(&mut self, endpoint: u8, max_len: usize, timeout: Duration)
    -> Result<Vec<u8>>;
    async fn bulk_out(&mut self, endpoint: u8, data: &[u8], timeout: Duration) -> Result<usize>;
}

#[async_trait]
pub trait GpioOps: Send + Sync {
    fn set(&mut self, line: u32, high: bool) -> Result<()>;
    fn pulse(&mut self, line: u32, duration: Duration) -> Result<()>;
    fn read(&mut self, _line: u32) -> Result<bool> {
        Err(crate::error::TransportError::UnsupportedCapability(
            "gpio.read",
        ))
    }
}

#[async_trait]
pub trait PsuOps: Send + Sync {
    async fn set_voltage(&mut self, channel: u8, volts: f32) -> Result<()>;
    async fn enable(&mut self, channel: u8) -> Result<()>;
    async fn disable(&mut self, channel: u8) -> Result<()>;
}

#[async_trait]
pub trait LogicAnalyzerOps: Send + Sync {
    async fn arm(&mut self) -> Result<()>;
    async fn capture(&mut self) -> Result<Vec<u8>>;
}
