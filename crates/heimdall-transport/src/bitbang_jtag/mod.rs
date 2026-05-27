//! Bit-banged JTAG TAP driver. Generic over `T: Transport + GpioOps`.

mod scan;
pub mod state;

use std::time::Duration;

use async_trait::async_trait;

use crate::caps::{GpioOps, JtagOps};
use crate::traits::{ResetTarget, Result, Transport, TransportKind};

#[derive(Debug, Clone, Copy)]
pub struct BitbangPins {
    pub tck: u32,
    pub tms: u32,
    pub tdi: u32,
    pub tdo: u32,
}

pub struct BitbangJtagTransport<T>
where
    T: Transport + GpioOps + Send + Sync,
{
    pub backend: T,
    pub pins: BitbangPins,
    pub clock_delay: Duration,
    pub tap: state::TapState,
    pub ir_width: u8,
}

impl<T> BitbangJtagTransport<T>
where
    T: Transport + GpioOps + Send + Sync,
{
    pub fn new(backend: T, pins: BitbangPins) -> Self {
        Self {
            backend,
            pins,
            clock_delay: Duration::from_micros(5),
            tap: state::TapState::TestLogicReset,
            ir_width: 4, // Aegis default. River callers override to 5.
        }
    }

    pub fn with_clock_delay(mut self, d: Duration) -> Self {
        self.clock_delay = d;
        self
    }

    pub fn with_ir_width(mut self, ir_width: u8) -> Self {
        assert!((1..=32).contains(&ir_width), "ir_width must be 1..=32");
        self.ir_width = ir_width;
        self
    }
}

#[async_trait]
impl<T> Transport for BitbangJtagTransport<T>
where
    T: Transport + GpioOps + Send + Sync,
{
    fn kind(&self) -> TransportKind {
        TransportKind::Jtag
    }

    async fn open(&mut self) -> Result<()> {
        self.backend.open().await?;
        Ok(())
    }

    async fn close(&mut self) -> Result<()> {
        self.backend.close().await
    }

    async fn reset(&mut self, _target: ResetTarget) -> Result<()> {
        scan::tap_reset(self)
    }
}

#[async_trait]
impl<T> JtagOps for BitbangJtagTransport<T>
where
    T: Transport + GpioOps + Send + Sync,
{
    async fn scan_idcode(&mut self) -> Result<Vec<u32>> {
        scan::scan_idcode(self)
    }

    async fn shift_dr(&mut self, ir: u32, bits: usize, data: &[u8]) -> Result<Vec<u8>> {
        scan::shift_ir_dr(self, ir, bits, data)
    }
}
