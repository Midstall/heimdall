use async_trait::async_trait;
use std::collections::VecDeque;
use std::time::Duration;

use crate::caps::{GpioOps, JtagOps, SerialOps};
use crate::error::TransportError;
use crate::traits::{ResetTarget, Result, Transport, TransportKind};

/// Scripted transport for tests. SerialOps reads from a queued buffer,
/// JtagOps::scan_idcode returns a configured chain, GpioOps records calls.
#[derive(Debug, Default)]
pub struct MockTransport {
    open: bool,
    pub serial_in: VecDeque<u8>,
    pub serial_out: Vec<u8>,
    pub idcode_chain: Vec<u32>,
    pub gpio_log: Vec<(u32, bool)>,
    pub resets: Vec<ResetTarget>,
    pub tdo_in: VecDeque<bool>,
}

impl MockTransport {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_serial_in(mut self, bytes: impl IntoIterator<Item = u8>) -> Self {
        self.serial_in.extend(bytes);
        self
    }

    pub fn with_idcode_chain(mut self, chain: impl IntoIterator<Item = u32>) -> Self {
        self.idcode_chain.extend(chain);
        self
    }

    pub fn with_tdo_in(mut self, bits: impl IntoIterator<Item = bool>) -> Self {
        self.tdo_in.extend(bits);
        self
    }
}

#[async_trait]
impl Transport for MockTransport {
    fn kind(&self) -> TransportKind {
        TransportKind::Mock
    }
    async fn open(&mut self) -> Result<()> {
        self.open = true;
        Ok(())
    }
    async fn close(&mut self) -> Result<()> {
        self.open = false;
        Ok(())
    }
    async fn reset(&mut self, target: ResetTarget) -> Result<()> {
        self.ensure_open()?;
        self.resets.push(target);
        Ok(())
    }
}

impl MockTransport {
    /// Set the transport to the open state without going through the async
    /// `Transport::open`. Useful in tests that exercise GpioOps directly.
    pub fn open_for_test(&mut self) {
        self.open = true;
    }

    fn ensure_open(&self) -> Result<()> {
        if !self.open {
            return Err(TransportError::NotOpen);
        }
        Ok(())
    }
}

#[async_trait]
impl SerialOps for MockTransport {
    async fn write_all(&mut self, bytes: &[u8]) -> Result<()> {
        self.ensure_open()?;
        self.serial_out.extend_from_slice(bytes);
        Ok(())
    }
    async fn read_until(&mut self, delim: u8, _timeout: Duration) -> Result<Vec<u8>> {
        self.ensure_open()?;
        let mut out = Vec::new();
        while let Some(b) = self.serial_in.pop_front() {
            out.push(b);
            if b == delim {
                return Ok(out);
            }
        }
        Err(TransportError::Timeout { millis: 0 })
    }
}

#[async_trait]
impl JtagOps for MockTransport {
    async fn scan_idcode(&mut self) -> Result<Vec<u32>> {
        self.ensure_open()?;
        Ok(self.idcode_chain.clone())
    }
    async fn shift_dr(&mut self, _ir: u32, _bits: usize, data: &[u8]) -> Result<Vec<u8>> {
        self.ensure_open()?;
        Ok(data.to_vec())
    }
}

#[async_trait]
impl GpioOps for MockTransport {
    fn set(&mut self, line: u32, high: bool) -> Result<()> {
        self.gpio_log.push((line, high));
        Ok(())
    }
    fn pulse(&mut self, line: u32, _duration: Duration) -> Result<()> {
        self.gpio_log.push((line, true));
        self.gpio_log.push((line, false));
        Ok(())
    }

    fn read(&mut self, _line: u32) -> Result<bool> {
        Ok(self.tdo_in.pop_front().unwrap_or(false))
    }
}
