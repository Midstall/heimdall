//! Integration test for AegisFpgaDriver against the current stub
//! load/run/observe paths. Asserts the wiring: driver returns empty State,
//! golden returns 128 io_N pads, the diff reports DiffMismatch on the first
//! golden pad. Revisit to expect Verdict::Pass once real bitstream load and
//! IO readback land.

#![cfg(feature = "aegis")]

use async_trait::async_trait;
use heimdall_core::{
    Artifact, ArtifactKind, DutId, DutKind, FailureKind, State, StepBudget, Verdict,
};
use heimdall_driver::{Dut, aegis::AegisFpgaDriver};
use heimdall_golden::AegisGoldenModel;
use heimdall_test::{BuildCtx, Plan, Runner, Test, TestError};
use heimdall_transport::mock::MockTransport;

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

struct EmptyExpectTest;

#[async_trait]
impl Test for EmptyExpectTest {
    fn name(&self) -> &str {
        "aegis-empty-expect"
    }
    fn target(&self) -> DutKind {
        DutKind::AegisLuna1
    }
    async fn build(&self, _ctx: &mut BuildCtx<'_>) -> Result<Plan, TestError> {
        // expected: empty State (matches the stubbed driver observation)
        // input: packed aegis image (descriptor + empty bitstream)
        let packed =
            AegisGoldenModel::pack_image(minimal_descriptor_json().as_bytes(), &[0u8; 128]);
        Ok(Plan {
            input: Artifact::new(ArtifactKind::RawBytes, packed),
            expected: State::new(),
            budget: StepBudget::cycles(10),
            inputs: std::collections::BTreeMap::new(),
        })
    }
}

#[tokio::test]
async fn aegis_mock_pipeline_fails_on_stubbed_observation() {
    // The pipeline runs end-to-end but the driver returns an empty State
    // while the golden returns 128 io_N pads. The first diff (vs the test's
    // empty expectation) passes; the second diff (vs the populated golden)
    // fails on io_0. The Runner reports the second-stage failure.
    let runner = Runner::builder().build();
    let mock_jtag = MockTransport::new();
    let mut driver = AegisFpgaDriver::new(DutKind::AegisLuna1, mock_jtag);
    let mut golden = AegisGoldenModel::new(DutKind::AegisLuna1);
    let mut dut = Dut::new(DutId::new("aegis-luna1-1"), DutKind::AegisLuna1);

    let res = runner
        .run_one(&EmptyExpectTest, &mut dut, &mut driver, &mut golden)
        .await
        .unwrap();

    // Driver stub returns empty State; golden returns 128 io_N pads.
    // Diff against golden flags the first pad as missing.
    match res.verdict {
        Verdict::Fail { kind, .. } => match kind {
            FailureKind::DiffMismatch { field, .. } => {
                assert!(
                    field.starts_with("io_"),
                    "expected io_N field, got `{field}`"
                );
            }
            other => panic!("expected DiffMismatch, got {other:?}"),
        },
        other => panic!("expected Fail (stub returns empty State), got {other:?}"),
    }
}
