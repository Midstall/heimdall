//! Verifies AegisFpgaDriver::load issues the CONFIG IR and shifts the
//! expected number of bits via DR, observed through a MockTransport.

#![cfg(feature = "aegis")]

use heimdall_core::{Artifact, ArtifactKind, BitstreamFormat, DutId, DutKind};
use heimdall_driver::aegis::{AegisFpgaDriver, jtag as aegis_jtag};
use heimdall_driver::{Dut, TestDriver};
use heimdall_golden::AegisGoldenModel;
use heimdall_transport::bitbang_jtag::{BitbangJtagTransport, BitbangPins};
use heimdall_transport::mock::MockTransport;

fn pins() -> BitbangPins {
    BitbangPins {
        tck: 0,
        tms: 1,
        tdi: 2,
        tdo: 3,
    }
}

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

#[tokio::test]
async fn load_shifts_config_ir_then_total_bits() {
    let backend = MockTransport::new();
    let jtag = BitbangJtagTransport::new(backend, pins())
        .with_clock_delay(std::time::Duration::from_nanos(1));
    let mut driver = AegisFpgaDriver::new(DutKind::AegisLuna1, jtag);

    // 233 bits -> ceil(233/8) = 30 bytes
    let bitstream = vec![0u8; 233usize.div_ceil(8)];
    let packed = AegisGoldenModel::pack_image(minimal_descriptor_json().as_bytes(), &bitstream);
    let artifact = Artifact::new(
        ArtifactKind::Bitstream {
            format: BitstreamFormat::AegisRaw,
        },
        packed,
    );

    let mut dut = Dut::new(DutId::new("luna1-1"), DutKind::AegisLuna1);
    driver.prepare(&mut dut).await.expect("prepare");
    driver.load(&mut dut, &artifact).await.expect("load");

    // Verify a non-empty GPIO sequence was driven.
    // prepare() -> tap_reset(): 6 clocks = 24 set() calls
    // load() -> shift_ir_dr():
    //   tap_reset():       6 clocks  = 24 set calls
    //   goto_shift_ir():   4 clocks  = 16 set calls
    //   shift IR (4 bits): 4 clocks  = 16 set calls
    //   goto_idle():       2 clocks  =  8 set calls
    //   goto_shift_dr():   3 clocks  = 12 set calls
    //   shift DR (233 b): 233 clocks = 932 set calls
    //   goto_idle():       2 clocks  =  8 set calls
    // load subtotal: 254 clocks = 1016 set calls
    // grand total:   260 clocks = 1040 set calls
    let total_clocks = driver.jtag.backend.gpio_log.len() / 4;
    assert!(
        total_clocks >= 250,
        "expected >= 250 clocks, got {total_clocks}"
    );

    driver.release(&mut dut).await.expect("release");
}

#[tokio::test]
async fn load_rejects_truncated_bitstream() {
    let backend = MockTransport::new();
    let jtag = BitbangJtagTransport::new(backend, pins())
        .with_clock_delay(std::time::Duration::from_nanos(1));
    let mut driver = AegisFpgaDriver::new(DutKind::AegisLuna1, jtag);

    // Provide an empty bitstream against a 233-bit descriptor.
    let packed = AegisGoldenModel::pack_image(minimal_descriptor_json().as_bytes(), &[]);
    let artifact = Artifact::new(ArtifactKind::RawBytes, packed);

    let mut dut = Dut::new(DutId::new("luna1-1"), DutKind::AegisLuna1);
    driver.prepare(&mut dut).await.expect("prepare");
    let err = driver
        .load(&mut dut, &artifact)
        .await
        .expect_err("expected truncation error");
    let msg = format!("{err}");
    assert!(msg.contains("shorter than descriptor"), "got `{msg}`");
}

#[test]
fn jtag_constants_match_spec() {
    assert_eq!(aegis_jtag::IR_WIDTH, 4);
    assert_eq!(aegis_jtag::IR_IDCODE, 0x1);
    assert_eq!(aegis_jtag::IR_CONFIG, 0x2);
    assert_eq!(aegis_jtag::IR_USER, 0x3);
    assert_eq!(aegis_jtag::IR_BYPASS, 0xF);
}
