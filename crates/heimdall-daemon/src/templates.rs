//! Campaign templates: build per-DUT-family job sequences.
//!
//! Template dispatcher selects per-family payloads:
//! - Aegis (FPGA): RunAegisVector with an all-zero bitstream and no expected outputs
//! - River (CPU):  BootRiverElf with a precompiled hello.elf (li a0, 0x42; ebreak)
//!
//! When a BringupPayload is present (resolved from heimdall.toml), its fields
//! are used instead of the MINIMAL_* fallback constants. When None, the
//! existing minimal hardcoded payloads are used (same as pre-v2 behavior).

use base64::Engine;
use heimdall_core::{DutId, DutKind, kind::Family};
use std::collections::BTreeMap;

use crate::dut_registry::BringupPayload;
use crate::types::{CampaignId, CampaignTemplate, JobKind, NewJob};

/// Top-level dispatch: returns the ordered list of NewJobs for a template.
pub fn build(
    template: &CampaignTemplate,
    dut: DutId,
    kind: DutKind,
    bringup: Option<&BringupPayload>,
    campaign: CampaignId,
) -> Vec<NewJob> {
    match template {
        CampaignTemplate::BringUp => bring_up(dut, kind, bringup, campaign),
        CampaignTemplate::Characterization => characterization(dut, kind, bringup, campaign),
        CampaignTemplate::Release => release(dut, kind, bringup, campaign),
        CampaignTemplate::Custom { .. } => Vec::new(),
    }
}

pub fn bring_up(
    dut: DutId,
    kind: DutKind,
    bringup: Option<&BringupPayload>,
    campaign: CampaignId,
) -> Vec<NewJob> {
    vec![family_bringup_job(dut, kind, bringup, campaign, 1)]
}

pub fn characterization(
    dut: DutId,
    kind: DutKind,
    bringup: Option<&BringupPayload>,
    campaign: CampaignId,
) -> Vec<NewJob> {
    // Three identical-shape jobs with increasing budgets to exercise more cycles.
    vec![
        family_bringup_job(dut.clone(), kind, bringup, campaign, 1),
        family_bringup_job(dut.clone(), kind, bringup, campaign, 10),
        family_bringup_job(dut, kind, bringup, campaign, 100),
    ]
}

pub fn release(
    dut: DutId,
    kind: DutKind,
    bringup: Option<&BringupPayload>,
    campaign: CampaignId,
) -> Vec<NewJob> {
    let mut jobs = characterization(dut.clone(), kind, bringup, campaign);
    // Append a final sign-off run with the longest budget.
    jobs.push(family_bringup_job(dut, kind, bringup, campaign, 1000));
    jobs
}

fn family_bringup_job(
    dut: DutId,
    kind: DutKind,
    bringup: Option<&BringupPayload>,
    campaign: CampaignId,
    budget_multiplier: u64,
) -> NewJob {
    match kind.family() {
        Family::Fpga => aegis_bringup_job(dut, bringup, campaign, budget_multiplier),
        Family::Cpu => river_bringup_job(dut, bringup, campaign, budget_multiplier),
    }
}

fn aegis_bringup_job(
    dut: DutId,
    bringup: Option<&BringupPayload>,
    campaign: CampaignId,
    budget_multiplier: u64,
) -> NewJob {
    let (descriptor_json, bitstream, inputs, expected_outputs, settle_cycles) = match bringup {
        Some(BringupPayload::AegisVector {
            descriptor_json,
            bitstream,
            inputs,
            expected_outputs,
            settle_cycles,
            ..
        }) => (
            descriptor_json.clone(),
            bitstream.clone(),
            inputs.clone(),
            expected_outputs.clone(),
            settle_cycles.saturating_mul(budget_multiplier).max(1),
        ),
        _ => (
            MINIMAL_AEGIS_DESCRIPTOR.to_string(),
            MINIMAL_AEGIS_BITSTREAM.to_vec(),
            BTreeMap::new(),
            BTreeMap::new(),
            budget_multiplier.max(1),
        ),
    };
    NewJob {
        dut,
        kind: JobKind::RunAegisVector {
            descriptor_json,
            bitstream_b64: base64::engine::general_purpose::STANDARD.encode(&bitstream),
            inputs,
            expected_outputs,
            settle_cycles,
        },
        campaign: Some(campaign),
    }
}

fn river_bringup_job(
    dut: DutId,
    bringup: Option<&BringupPayload>,
    campaign: CampaignId,
    budget_multiplier: u64,
) -> NewJob {
    let (elf, base_cycles) = match bringup {
        Some(BringupPayload::RiverElf { elf, cycles, .. }) => (elf.clone(), *cycles),
        _ => (RIVER_BRINGUP_ELF.to_vec(), 1_000),
    };
    NewJob {
        dut,
        kind: JobKind::BootRiverElf {
            elf_b64: base64::engine::general_purpose::STANDARD.encode(&elf),
            cycles: base_cycles.saturating_mul(budget_multiplier).max(1),
        },
        campaign: Some(campaign),
    }
}

// ---------------------------------------------------------------------------
// Minimal hardcoded payloads.

/// Minimal Aegis device descriptor matching the shape used by aegis-ip's own
/// tests: 2x2 fabric, 1 track, 233 total config bits. Placeholder for
/// bring-up runs. Replace with the real per-device descriptor in production
/// deployments.
const MINIMAL_AEGIS_DESCRIPTOR: &str = r#"{
    "device": "test_fpga",
    "fabric": {
        "width": 2, "height": 2, "tracks": 1, "tile_config_width": 46,
        "bram": {"column_interval":0,"columns":[],"data_width":null,"addr_width":null,"depth":null,"tile_config_width":8},
        "dsp":  {"column_interval":0,"columns":[],"a_width":null,"b_width":null,"result_width":null,"tile_config_width":16},
        "carry_chain": {"direction":"south_to_north","per_column":true}
    },
    "io": {"total_pads": 8, "tile_config_width": 8, "pads": []},
    "serdes": {"count":0,"tile_config_width":32,"edge_assignment":[]},
    "clock":  {"tile_count":1,"tile_config_width":49,"outputs_per_tile":4,"total_outputs":4},
    "config": {"total_bits":233,"chain_order":[]},
    "tiles": []
}"#;

/// All-zero bitstream sized for the minimal descriptor's 233 total bits
/// (rounded up to 30 bytes). Encodes "no logic enabled, outputs floating".
const MINIMAL_AEGIS_BITSTREAM: [u8; 30] = [0u8; 30];

/// Precompiled RV64 ELF that loads 0x42 into a0 and hits ebreak, then loops.
/// Built from testdata/river-bringup/hello.S via riscv64-none-elf-as + ld.
const RIVER_BRINGUP_ELF: &[u8] = include_bytes!("../../../testdata/river-bringup/hello.elf");

#[cfg(test)]
mod tests {
    use super::*;
    use heimdall_core::DutKind;

    #[test]
    fn bringup_aegis_produces_run_aegis_vector() {
        let jobs = bring_up(
            DutId::new("luna1-1"),
            DutKind::AegisLuna1,
            None,
            CampaignId::new(),
        );
        assert_eq!(jobs.len(), 1);
        match &jobs[0].kind {
            JobKind::RunAegisVector {
                descriptor_json,
                bitstream_b64,
                settle_cycles,
                ..
            } => {
                assert!(descriptor_json.contains("\"device\": \"test_fpga\""));
                let bs = base64::engine::general_purpose::STANDARD
                    .decode(bitstream_b64.as_bytes())
                    .unwrap();
                assert_eq!(bs.len(), 30);
                assert_eq!(*settle_cycles, 1);
            }
            other => panic!("expected RunAegisVector, got {other:?}"),
        }
    }

    #[test]
    fn bringup_river_produces_boot_river_elf() {
        let jobs = bring_up(
            DutId::new("river-1"),
            DutKind::RiverRc1Nano,
            None,
            CampaignId::new(),
        );
        assert_eq!(jobs.len(), 1);
        match &jobs[0].kind {
            JobKind::BootRiverElf { elf_b64, cycles } => {
                let elf = base64::engine::general_purpose::STANDARD
                    .decode(elf_b64.as_bytes())
                    .unwrap();
                // ELF magic: 7F 45 4C 46
                assert_eq!(&elf[..4], b"\x7fELF");
                assert_eq!(*cycles, 1_000);
            }
            other => panic!("expected BootRiverElf, got {other:?}"),
        }
    }

    #[test]
    fn characterization_produces_three_jobs() {
        for kind in [DutKind::AegisLuna1, DutKind::RiverRc1Small] {
            let jobs = characterization(DutId::new("d"), kind, None, CampaignId::new());
            assert_eq!(jobs.len(), 3, "{kind:?} characterization");
        }
    }

    #[test]
    fn release_produces_four_jobs() {
        let jobs = release(
            DutId::new("luna1-1"),
            DutKind::AegisLuna1,
            None,
            CampaignId::new(),
        );
        assert_eq!(jobs.len(), 4);
    }

    #[test]
    fn dispatch_routes_by_variant() {
        let cid = CampaignId::new();
        let dut = DutId::new("d1");
        for t in [
            CampaignTemplate::BringUp,
            CampaignTemplate::Characterization,
            CampaignTemplate::Release,
        ] {
            for kind in [DutKind::AegisLuna1, DutKind::RiverRc1Nano] {
                let jobs = build(&t, dut.clone(), kind, None, cid);
                assert!(!jobs.is_empty(), "{t:?}/{kind:?} produced no jobs");
            }
        }
        let custom = CampaignTemplate::Custom {
            name: "anything".into(),
        };
        assert!(build(&custom, dut, DutKind::RiverRc1Nano, None, cid).is_empty());
    }

    #[test]
    fn bringup_aegis_uses_bringup_payload_when_present() {
        let mut inputs = BTreeMap::new();
        inputs.insert("io_0".into(), true);
        let mut expected = BTreeMap::new();
        expected.insert("io_2".into(), true);
        let payload = BringupPayload::AegisVector {
            descriptor_json: "{\"device\":\"custom\"}".into(),
            bitstream: vec![0xaa, 0xbb, 0xcc],
            bitstream_len: 3,
            inputs: inputs.clone(),
            expected_outputs: expected.clone(),
            settle_cycles: 7,
        };
        let jobs = bring_up(
            DutId::new("luna1-1"),
            DutKind::AegisLuna1,
            Some(&payload),
            CampaignId::new(),
        );
        match &jobs[0].kind {
            JobKind::RunAegisVector {
                descriptor_json,
                bitstream_b64,
                inputs: got_inputs,
                expected_outputs: got_expected,
                settle_cycles,
            } => {
                assert_eq!(descriptor_json, "{\"device\":\"custom\"}");
                let bs = base64::engine::general_purpose::STANDARD
                    .decode(bitstream_b64.as_bytes())
                    .unwrap();
                assert_eq!(bs, vec![0xaa, 0xbb, 0xcc]);
                assert_eq!(got_inputs, &inputs);
                assert_eq!(got_expected, &expected);
                // multiplier is 1 in bring_up. payload settle_cycles=7 * 1 = 7.
                assert_eq!(*settle_cycles, 7);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn bringup_river_uses_bringup_payload_when_present() {
        let payload = BringupPayload::RiverElf {
            elf: b"\x7fELF_CUSTOM".to_vec(),
            elf_len: 11,
            cycles: 50,
        };
        let jobs = bring_up(
            DutId::new("river-1"),
            DutKind::RiverRc1Nano,
            Some(&payload),
            CampaignId::new(),
        );
        match &jobs[0].kind {
            JobKind::BootRiverElf { elf_b64, cycles } => {
                let elf = base64::engine::general_purpose::STANDARD
                    .decode(elf_b64.as_bytes())
                    .unwrap();
                assert_eq!(elf, b"\x7fELF_CUSTOM");
                // multiplier 1. payload cycles 50 * 1 = 50.
                assert_eq!(*cycles, 50);
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}
