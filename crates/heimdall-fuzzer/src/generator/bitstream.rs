//! Bitstream generator for Aegis FPGA targets. Pairs with `AegisGoldenModel`
//! (heimdall-golden) for differential testing.

use aegis_ip::AegisFpgaDeviceDescriptor;
use heimdall_core::{Artifact, ArtifactKind, BitstreamFormat, DutKind, SeedId};
use heimdall_golden::AegisGoldenModel;
use rand::RngCore;

use crate::traits::Generator;

pub struct BitstreamGen {
    target: DutKind,
    /// Serialized descriptor JSON (cached so we don't re-serialize each gen).
    descriptor_json: Vec<u8>,
    /// Number of bitstream bytes to generate per artifact.
    bitstream_bytes: usize,
}

impl BitstreamGen {
    /// Build a generator from a parsed descriptor. The descriptor JSON used
    /// for packing comes from re-serializing the parsed value. For byte-exact
    /// round-trip with the user's original JSON, use `with_descriptor_json`.
    pub fn new(target: DutKind, descriptor: &AegisFpgaDeviceDescriptor) -> Self {
        let descriptor_json = serde_json::to_vec(descriptor).expect("descriptor reserialize");
        let total_bits = descriptor.config.total_bits as usize;
        let bitstream_bytes = total_bits.div_ceil(8);
        Self {
            target,
            descriptor_json,
            bitstream_bytes,
        }
    }

    pub fn with_descriptor_json(
        target: DutKind,
        descriptor_json: Vec<u8>,
        bitstream_bytes: usize,
    ) -> Self {
        Self {
            target,
            descriptor_json,
            bitstream_bytes,
        }
    }
}

impl Generator for BitstreamGen {
    fn target(&self) -> DutKind {
        self.target
    }

    fn name(&self) -> &str {
        "bitstream-aegis"
    }

    fn generate(&mut self, rng: &mut dyn RngCore, _seed: SeedId) -> Artifact {
        let mut bitstream = vec![0u8; self.bitstream_bytes];
        rng.fill_bytes(&mut bitstream);
        let packed = AegisGoldenModel::pack_image(&self.descriptor_json, &bitstream);
        Artifact::new(
            ArtifactKind::Bitstream {
                format: BitstreamFormat::AegisRaw,
            },
            packed,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use heimdall_core::StepBudget;
    use heimdall_golden::AegisGoldenModel;
    use heimdall_golden::GoldenModel;
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    fn minimal_descriptor_json() -> &'static str {
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

    fn descriptor() -> AegisFpgaDeviceDescriptor {
        serde_json::from_str(minimal_descriptor_json()).unwrap()
    }

    #[test]
    fn produces_bitstream_artifact_kind() {
        let mut g = BitstreamGen::new(DutKind::AegisLuna1, &descriptor());
        let a = g.generate(&mut StdRng::seed_from_u64(1), SeedId(0));
        match a.kind {
            ArtifactKind::Bitstream { format } => {
                assert_eq!(format, BitstreamFormat::AegisRaw);
            }
            other => panic!("unexpected kind {other:?}"),
        }
    }

    #[test]
    fn deterministic_for_same_rng_seed() {
        let mut g1 = BitstreamGen::new(DutKind::AegisLuna1, &descriptor());
        let mut g2 = BitstreamGen::new(DutKind::AegisLuna1, &descriptor());
        let a = g1.generate(&mut StdRng::seed_from_u64(7), SeedId(0));
        let b = g2.generate(&mut StdRng::seed_from_u64(7), SeedId(0));
        assert_eq!(a.bytes, b.bytes);
    }

    #[tokio::test]
    async fn output_is_loadable_by_aegis_golden_model() {
        let mut g = BitstreamGen::new(DutKind::AegisLuna1, &descriptor());
        let a = g.generate(&mut StdRng::seed_from_u64(2), SeedId(0));
        let mut model = AegisGoldenModel::new(DutKind::AegisLuna1);
        model.load(&a).await.expect("loadable");
        // Step a few cycles to confirm the sim doesn't choke on random bits.
        let _ = model.step(StepBudget::cycles(2)).await.expect("step");
    }
}
