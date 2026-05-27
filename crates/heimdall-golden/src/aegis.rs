//! AegisGoldenModel: wraps aegis_sim::Simulator as a heimdall GoldenModel.
//!
//! Image layout (Artifact bytes):
//!   - 4 bytes little-endian u32 `descriptor_len`
//!   - `descriptor_len` bytes of UTF-8 JSON: AegisFpgaDeviceDescriptor
//!   - remainder: raw bitstream bytes for aegis_sim::Simulator
//!
//! This container is documented and small. A future plan introduces a typed
//! `ArtifactKind::AegisImage` if/when we want native serde for it.

use std::convert::TryFrom;

use aegis_ip::AegisFpgaDeviceDescriptor;
use aegis_sim::Simulator;
use async_trait::async_trait;
use heimdall_core::{Artifact, DutKind, State, StepBudget, ValueRepr};

use crate::error::GoldenError;
use crate::trait_def::{GoldenModel, Result, StepOutcome};

/// Maximum number of IO pads emitted by `observe()`. `get_io` returns `false`
/// for out-of-range indices so this is safe for any device with fewer pads.
const MAX_OBSERVE_PADS: usize = 128;

pub struct AegisGoldenModel {
    target: DutKind,
    sim: Option<Simulator>,
}

impl AegisGoldenModel {
    pub fn new(target: DutKind) -> Self {
        Self { target, sim: None }
    }

    /// Build an Artifact bytes payload from a descriptor JSON and a bitstream.
    /// Useful in tests and from external callers.
    pub fn pack_image(descriptor_json: &[u8], bitstream: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 + descriptor_json.len() + bitstream.len());
        let len = u32::try_from(descriptor_json.len()).expect("descriptor under 4GiB");
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(descriptor_json);
        out.extend_from_slice(bitstream);
        out
    }

    /// Drive an input pad. The simulator must be loaded.
    pub fn set_io(&mut self, pad: usize, value: bool) -> Result<()> {
        let sim = self.sim.as_mut().ok_or(GoldenError::NotLoaded)?;
        sim.set_io(pad, value);
        Ok(())
    }
}

fn parse_image(bytes: &[u8]) -> Result<(AegisFpgaDeviceDescriptor, &[u8])> {
    if bytes.len() < 4 {
        return Err(GoldenError::ParseSpike("image too short for header".into()));
    }
    let mut len_buf = [0u8; 4];
    len_buf.copy_from_slice(&bytes[..4]);
    let desc_len = u32::from_le_bytes(len_buf) as usize;
    if bytes.len() < 4 + desc_len {
        return Err(GoldenError::ParseSpike(
            "image truncated before descriptor end".into(),
        ));
    }
    let desc_bytes = &bytes[4..4 + desc_len];
    let desc: AegisFpgaDeviceDescriptor = serde_json::from_slice(desc_bytes)
        .map_err(|e| GoldenError::ParseSpike(format!("descriptor JSON: {e}")))?;
    let bitstream = &bytes[4 + desc_len..];
    Ok((desc, bitstream))
}

#[async_trait]
impl GoldenModel for AegisGoldenModel {
    fn target(&self) -> DutKind {
        self.target
    }

    async fn reset(&mut self) -> Result<()> {
        self.sim = None;
        Ok(())
    }

    async fn load(&mut self, image: &Artifact) -> Result<()> {
        let (desc, bitstream) = parse_image(&image.bytes)?;
        self.sim = Some(Simulator::new(&desc, bitstream));
        Ok(())
    }

    async fn step(&mut self, budget: StepBudget) -> Result<StepOutcome> {
        let sim = self.sim.as_mut().ok_or(GoldenError::NotLoaded)?;
        let cycles = match budget {
            StepBudget::Cycles { count } => count,
            other => return Err(GoldenError::UnsupportedBudget(other, "aegis-sim")),
        };
        sim.run(cycles);
        Ok(StepOutcome::RanFully)
    }

    async fn observe(&mut self) -> Result<State> {
        let sim = self.sim.as_ref().ok_or(GoldenError::NotLoaded)?;
        let mut state = State::new();
        for pad in 0..MAX_OBSERVE_PADS {
            let v = sim.get_io(pad);
            state = state.with(format!("io_{pad}"), ValueRepr::Bool(v));
        }
        Ok(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use heimdall_core::ArtifactKind;

    fn minimal_descriptor_json() -> &'static str {
        // Same fixture as aegis-ip's own test.
        r#"{
            "device": "test_fpga",
            "fabric": {
                "width": 2,
                "height": 2,
                "tracks": 1,
                "tile_config_width": 46,
                "bram": {
                    "column_interval": 0,
                    "columns": [],
                    "data_width": null,
                    "addr_width": null,
                    "depth": null,
                    "tile_config_width": 8
                },
                "dsp": {
                    "column_interval": 0,
                    "columns": [],
                    "a_width": null,
                    "b_width": null,
                    "result_width": null,
                    "tile_config_width": 16
                },
                "carry_chain": {
                    "direction": "south_to_north",
                    "per_column": true
                }
            },
            "io": {
                "total_pads": 8,
                "tile_config_width": 8,
                "pads": []
            },
            "serdes": {
                "count": 0,
                "tile_config_width": 32,
                "edge_assignment": []
            },
            "clock": {
                "tile_count": 1,
                "tile_config_width": 49,
                "outputs_per_tile": 4,
                "total_outputs": 4
            },
            "config": {
                "total_bits": 233,
                "chain_order": []
            },
            "tiles": []
        }"#
    }

    #[test]
    fn pack_image_layout() {
        let desc = b"abc";
        let bs = b"\x01\x02\x03\x04";
        let packed = AegisGoldenModel::pack_image(desc, bs);
        assert_eq!(&packed[..4], &3u32.to_le_bytes());
        assert_eq!(&packed[4..7], desc);
        assert_eq!(&packed[7..], bs);
    }

    #[test]
    fn parse_image_roundtrip() {
        let desc = minimal_descriptor_json().as_bytes();
        let bs = b"\x00\x00\x00";
        let packed = AegisGoldenModel::pack_image(desc, bs);
        let (parsed_desc, parsed_bs) = parse_image(&packed).unwrap();
        assert_eq!(parsed_desc.device, "test_fpga");
        assert_eq!(parsed_bs, bs);
    }

    #[tokio::test]
    async fn load_step_observe() {
        let mut model = AegisGoldenModel::new(DutKind::AegisLuna1);
        let packed =
            AegisGoldenModel::pack_image(minimal_descriptor_json().as_bytes(), &[0u8; 128]);
        let artifact = Artifact::new(ArtifactKind::RawBytes, packed);
        model.load(&artifact).await.expect("load");
        let outcome = model.step(StepBudget::cycles(10)).await.expect("step");
        assert_eq!(outcome, StepOutcome::RanFully);
        let state = model.observe().await.expect("observe");
        // We emit io_0..io_127. Assert the structure even though all values
        // are false for an empty bitstream.
        assert_eq!(state.fields.len(), 128);
        assert!(state.fields.contains_key("io_0"));
        assert!(state.fields.contains_key("io_127"));
    }

    #[tokio::test]
    async fn step_before_load_errors() {
        let mut model = AegisGoldenModel::new(DutKind::AegisLuna1);
        let err = model.step(StepBudget::cycles(1)).await.unwrap_err();
        assert!(matches!(err, GoldenError::NotLoaded));
    }
}
