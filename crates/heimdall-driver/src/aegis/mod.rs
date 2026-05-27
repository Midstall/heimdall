//! AegisFpgaDriver: TestDriver impl for Aegis FPGA silicon.
//!
//! Transport-generic over `T: Transport + JtagOps` so tests can drive a
//! MockTransport and real hardware can plug in a bit-banged or hardware
//! JTAG backend.

use aegis_ip::AegisFpgaDeviceDescriptor;
use async_trait::async_trait;
use heimdall_core::{Artifact, DutKind, Observation, State, Stimulus, Verdict};
use heimdall_transport::{GpioTransport, JtagOps, Transport, TransportKind};
use std::time::Instant;
use tracing::{instrument, warn};

use crate::error::DriverError;
use crate::trait_def::{Dut, Result, TestDriver};

pub mod diff;
pub mod jtag;
pub mod pinmap;

pub struct AegisFpgaDriver<T>
where
    T: Transport + JtagOps + Send + Sync,
{
    target: DutKind,
    pub jtag: T,
    pub expect_idcode: Option<u32>,
    pub pad_map: pinmap::IoPinmap,
    pub gpio: Option<Box<dyn GpioTransport>>,
    pub settle: std::time::Duration,
}

impl<T> AegisFpgaDriver<T>
where
    T: Transport + JtagOps + Send + Sync,
{
    pub fn new(target: DutKind, jtag: T) -> Self {
        Self {
            target,
            jtag,
            expect_idcode: None,
            pad_map: pinmap::IoPinmap::new(),
            gpio: None,
            settle: std::time::Duration::from_millis(10),
        }
    }

    pub fn with_expect_idcode(mut self, idcode: u32) -> Self {
        self.expect_idcode = Some(idcode);
        self
    }

    pub fn with_pinmap(mut self, gpio: Box<dyn GpioTransport>, map: pinmap::IoPinmap) -> Self {
        self.gpio = Some(gpio);
        self.pad_map = map;
        self
    }

    pub fn with_settle(mut self, d: std::time::Duration) -> Self {
        self.settle = d;
        self
    }

    fn observe_internal(&mut self) -> Result<State> {
        let mut state = State::new();
        // Collect outputs first to avoid simultaneous mutable + immutable borrow of self.
        let outputs: Vec<pinmap::PadEntry> = self.pad_map.outputs().cloned().collect();
        if let Some(gpio) = self.gpio.as_mut() {
            for entry in outputs {
                let v = gpio.read(entry.gpio_line).map_err(DriverError::from)?;
                state = state.with(
                    format!("io_{}", entry.fpga_pad),
                    heimdall_core::ValueRepr::Bool(v),
                );
            }
        }
        Ok(state)
    }
}

#[async_trait]
impl<T> TestDriver for AegisFpgaDriver<T>
where
    T: Transport + JtagOps + Send + Sync,
{
    fn target(&self) -> DutKind {
        self.target
    }

    fn required_transports(&self) -> &[TransportKind] {
        const REQ: &[TransportKind] = &[TransportKind::Jtag];
        REQ
    }

    #[instrument(skip(self, _dut))]
    async fn prepare(&mut self, _dut: &mut Dut) -> Result<()> {
        self.jtag.open().await?;
        self.jtag
            .reset(heimdall_transport::ResetTarget::System)
            .await?;
        if let Some(expected) = self.expect_idcode {
            let chain = self.jtag.scan_idcode().await?;
            if let Some(got) = chain.first().copied() {
                if got != expected {
                    return Err(DriverError::IdcodeMismatch { got, expected });
                }
            }
        }
        if let Some(gpio) = self.gpio.as_mut() {
            gpio.open().await.map_err(DriverError::from)?;
        }
        Ok(())
    }

    async fn compile(
        &mut self,
        input: &Artifact,
        _tools: &heimdall_tools::ToolChain,
    ) -> Result<Artifact> {
        // "compile" for Aegis is a no-op today: callers pass a packed image
        // (descriptor + bitstream) directly. Higher-level inputs would route
        // through `aegis-pack` here.
        Ok(input.clone())
    }

    #[instrument(skip(self, _dut, image))]
    async fn load(&mut self, _dut: &mut Dut, image: &Artifact) -> Result<()> {
        // Parse the packed (descriptor_json, bitstream) container produced by
        // AegisGoldenModel::pack_image.
        let (descriptor, bitstream) = parse_image(&image.bytes)?;
        let total_bits = descriptor.config.total_bits as usize;
        let expected_bytes = total_bits.div_ceil(8);
        if bitstream.len() < expected_bytes {
            return Err(DriverError::State(
                "bitstream shorter than descriptor total_bits",
            ));
        }
        if bitstream.len() > expected_bytes {
            warn!(
                got = bitstream.len(),
                expected = expected_bytes,
                "bitstream longer than total_bits; extra bytes ignored"
            );
        }

        // Issue the CONFIG IR + bitstream DR shift.
        // shift_dr already issues: reset TAP -> shift IR -> idle -> shift DR -> idle.
        let _tdo = self
            .jtag
            .shift_dr(jtag::IR_CONFIG, total_bits, &bitstream[..expected_bytes])
            .await?;
        Ok(())
    }

    async fn run(&mut self, _dut: &mut Dut, stim: &Stimulus) -> Result<Observation> {
        let started = Instant::now();
        // Collect inputs first to avoid simultaneous mutable + immutable borrow.
        let inputs: Vec<pinmap::PadEntry> = self.pad_map.inputs().cloned().collect();
        if let Some(gpio) = self.gpio.as_mut() {
            for entry in inputs {
                let key = format!("io_{}", entry.fpga_pad);
                let value = stim.inputs.get(&key).copied().unwrap_or(false);
                gpio.set(entry.gpio_line, value)
                    .map_err(DriverError::from)?;
            }
        } else if !self.pad_map.is_empty() {
            tracing::warn!("pinmap configured but no gpio transport set; inputs not driven");
        }
        // Let any combinational/sequential paths settle.
        tokio::time::sleep(self.settle).await;
        // Snapshot via observe_internal so run's Observation matches the next observe().
        let state = self.observe_internal()?;
        Ok(Observation::new(state, started.elapsed()))
    }

    async fn observe(&mut self, _dut: &mut Dut) -> Result<State> {
        self.observe_internal()
    }

    async fn diff(&self, dut_state: &State, golden_state: &State) -> Verdict {
        diff::diff_states(dut_state, golden_state)
    }

    async fn release(&mut self, _dut: &mut Dut) -> Result<()> {
        if let Some(gpio) = self.gpio.as_mut() {
            let _ = gpio.close().await; // best-effort; jtag close is primary
        }
        self.jtag.close().await?;
        Ok(())
    }
}

fn parse_image(bytes: &[u8]) -> Result<(AegisFpgaDeviceDescriptor, &[u8])> {
    if bytes.len() < 4 {
        return Err(DriverError::State("image too short for header"));
    }
    let mut len_buf = [0u8; 4];
    len_buf.copy_from_slice(&bytes[..4]);
    let desc_len = u32::from_le_bytes(len_buf) as usize;
    if bytes.len() < 4 + desc_len {
        return Err(DriverError::State("image truncated before descriptor end"));
    }
    let desc_bytes = &bytes[4..4 + desc_len];
    let desc: AegisFpgaDeviceDescriptor =
        serde_json::from_slice(desc_bytes).map_err(|_| DriverError::State("descriptor parse"))?;
    let bitstream = &bytes[4 + desc_len..];
    Ok((desc, bitstream))
}
