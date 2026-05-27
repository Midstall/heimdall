//! Integration test for BitbangJtagTransport against MockTransport.

#![cfg(feature = "bitbang-jtag")]

use heimdall_transport::bitbang_jtag::{BitbangJtagTransport, BitbangPins};
use heimdall_transport::mock::MockTransport;
use heimdall_transport::{JtagOps, Transport};

fn pins() -> BitbangPins {
    BitbangPins {
        tck: 0,
        tms: 1,
        tdi: 2,
        tdo: 3,
    }
}

fn idcode_bits_lsb(idcode: u32) -> Vec<bool> {
    (0..32).map(|i| (idcode >> i) & 1 == 1).collect()
}

/// scan_idcode calls tap_reset (6 clocks) then goto_shift_dr (3 clocks)
/// before reading 32 IDCODE bits. Pre-fill with dummy false bits for the
/// navigation phase so the IDCODE bits land in the right slots.
const SCAN_IDCODE_OVERHEAD_CLOCKS: usize = 6 + 3;

#[tokio::test]
async fn scan_idcode_reads_chain_value() {
    let want: u32 = 0xdead_beef;
    let mut tdo_in = vec![false; SCAN_IDCODE_OVERHEAD_CLOCKS];
    tdo_in.extend(idcode_bits_lsb(want));
    let backend = MockTransport::new().with_tdo_in(tdo_in);
    let mut jtag = BitbangJtagTransport::new(backend, pins())
        .with_clock_delay(std::time::Duration::from_nanos(1));
    jtag.open().await.unwrap();
    let chain = jtag.scan_idcode().await.unwrap();
    assert_eq!(chain, vec![want]);
    jtag.close().await.unwrap();
}

#[tokio::test]
async fn scan_idcode_filters_all_zeros() {
    // All zeros including overhead: result is empty chain.
    let backend = MockTransport::new().with_tdo_in(vec![false; SCAN_IDCODE_OVERHEAD_CLOCKS + 32]);
    let mut jtag = BitbangJtagTransport::new(backend, pins())
        .with_clock_delay(std::time::Duration::from_nanos(1));
    jtag.open().await.unwrap();
    let chain = jtag.scan_idcode().await.unwrap();
    assert!(chain.is_empty());
}

#[tokio::test]
async fn shift_dr_returns_tdo_bits() {
    // shift_ir_dr clocks:
    //   tap_reset:     6 clocks (5x TMS=1 + 1x TMS=0)
    //   goto_shift_ir: 4 clocks
    //   IR phase:      4 clocks  (ir_width default = 4, Aegis)
    //   goto_idle:     2 clocks
    //   goto_shift_dr: 3 clocks
    //   DR phase:     16 clocks  <- we care about these
    let overhead = 6 + 4 + 4 + 2 + 3;
    let mut tdo_in = vec![false; overhead];
    // DR phase: 16 bits alternating 1,0,1,0,... (LSB-first -> 0x55 per byte)
    let dr_bits: Vec<bool> = (0..16).map(|i| i % 2 == 0).collect();
    tdo_in.extend(dr_bits);

    let backend = MockTransport::new().with_tdo_in(tdo_in);
    let mut jtag = BitbangJtagTransport::new(backend, pins())
        .with_clock_delay(std::time::Duration::from_nanos(1));
    jtag.open().await.unwrap();
    let out = jtag.shift_dr(0, 16, &[0; 2]).await.unwrap();
    // bits [1,0,1,0,1,0,1,0] LSB-first = 0x55
    assert_eq!(out[0], 0x55);
    assert_eq!(out[1], 0x55);
    jtag.close().await.unwrap();
}
