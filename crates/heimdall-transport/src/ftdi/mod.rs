//! Pure-Rust FTDI MPSSE JTAG transport.
//!
//! Wraps `nusb` for USB I/O and an in-crate MPSSE command encoder for the
//! JTAG state machine. Replaces "install OpenOCD, write a tcl config" with
//! "plug in the FT2232H, set vid/pid in heimdall.toml, run."
//!
//! See `mpsse` for the protocol layer (unit-testable via `mock::MockMpsse`)
//! and `device` for the real `nusb` backend.

pub mod device;
pub mod mock;
pub mod mpsse;

use async_trait::async_trait;

use crate::caps::JtagOps;
use crate::traits::{ResetTarget, Result, Transport, TransportKind};

pub use device::{
    DEFAULT_IO_TIMEOUT, FtdiDeviceInfo, FtdiNusbBackend, FtdiUsbConfig, enumerate_ftdi_devices,
};
pub use mock::MockMpsse;
pub use mpsse::{MpsseBackend, MpsseJtag, TapState};

/// `JtagTransport`-compatible wrapper around a real or mock MPSSE backend.
///
/// Generic over `B: MpsseBackend` so tests can drive the full
/// `Transport + JtagOps` surface against `MockMpsse`.
pub struct FtdiJtagTransport<B: MpsseBackend + 'static> {
    jtag: MpsseJtag<B>,
    /// MPSSE TCK divisor. F_TCK = 6 MHz / (1 + divisor). 5 -> 1 MHz.
    divisor: u16,
    opened: bool,
}

impl<B: MpsseBackend + 'static> FtdiJtagTransport<B> {
    pub fn new(backend: B, ir_width: u8, divisor: u16) -> Self {
        Self {
            jtag: MpsseJtag::new(backend, ir_width),
            divisor,
            opened: false,
        }
    }

    pub fn with_divisor(mut self, divisor: u16) -> Self {
        self.divisor = divisor;
        self
    }

    pub fn jtag(&self) -> &MpsseJtag<B> {
        &self.jtag
    }

    pub fn jtag_mut(&mut self) -> &mut MpsseJtag<B> {
        &mut self.jtag
    }
}

impl FtdiJtagTransport<FtdiNusbBackend> {
    /// Open the real FTDI device identified by `cfg` and prepare it for JTAG.
    pub fn open_nusb(cfg: &FtdiUsbConfig, ir_width: u8, divisor: u16) -> Result<Self> {
        let backend = FtdiNusbBackend::open(cfg)?;
        let mut t = Self::new(backend, ir_width, divisor);
        t.jtag.init(divisor)?;
        t.opened = true;
        Ok(t)
    }
}

#[async_trait]
impl<B: MpsseBackend + 'static> Transport for FtdiJtagTransport<B> {
    fn kind(&self) -> TransportKind {
        TransportKind::Jtag
    }

    async fn open(&mut self) -> Result<()> {
        if !self.opened {
            self.jtag.init(self.divisor)?;
            self.opened = true;
        }
        Ok(())
    }

    async fn close(&mut self) -> Result<()> {
        self.opened = false;
        Ok(())
    }

    async fn reset(&mut self, _target: ResetTarget) -> Result<()> {
        self.jtag.reset_tap()
    }
}

#[async_trait]
impl<B: MpsseBackend + 'static> JtagOps for FtdiJtagTransport<B> {
    async fn scan_idcode(&mut self) -> Result<Vec<u32>> {
        self.jtag.scan_idcode()
    }

    async fn shift_dr(&mut self, ir: u32, bits: usize, data: &[u8]) -> Result<Vec<u8>> {
        self.jtag.shift_ir_dr(ir, bits, data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ftdi_transport_open_drives_init_then_idcode() {
        let mut backend = MockMpsse::new();
        // init() sync echo
        backend.queue_read([0xFA, 0xAA]);
        // scan_idcode reads 5 bytes. Canned IDCODE = 0x12345678.
        // 0x12345678 little-endian = 78 56 34 12.
        // 24-bit prefix: 78 56 34.
        // bits 24..30 = bits 0..6 of 0x12 = 0,1,0,0,1,0,0 → MSB-justified byte:
        //   (b6<<7)|(b5<<6)|(b4<<5)|(b3<<4)|(b2<<3)|(b1<<2)|(b0<<1)|0
        //   = (0<<7)|(0<<6)|(1<<5)|(0<<4)|(0<<3)|(1<<2)|(0<<1)|0
        //   = 0x24
        //   Verify: 0x24 >> 1 = 0x12. bits 0..6 of 0x12 = 0,1,0,0,1,0,0. yes
        // bit 31 = bit 7 of 0x12 = 0, so TMS byte = 0x00.
        backend.queue_read([0x78, 0x56, 0x34, 0x24, 0x00]);

        let mut t = FtdiJtagTransport::new(backend, 5, 5);
        t.open().await.expect("open");
        let codes = t.scan_idcode().await.expect("idcode");
        assert_eq!(codes, vec![0x12345678]);
        t.close().await.expect("close");
    }

    #[tokio::test]
    async fn ftdi_transport_kind_is_jtag() {
        let backend = MockMpsse::new();
        let t = FtdiJtagTransport::new(backend, 5, 5);
        assert_eq!(t.kind(), TransportKind::Jtag);
    }
}
