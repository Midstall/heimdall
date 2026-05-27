//! Verifies AegisFpgaDriver::observe reads back output pads via GpioOps.

#![cfg(feature = "aegis")]

use heimdall_core::{DutId, DutKind, ValueRepr};
use heimdall_driver::aegis::AegisFpgaDriver;
use heimdall_driver::aegis::pinmap::{IoPinmap, PadDirection, PadEntry};
use heimdall_driver::{Dut, TestDriver};
use heimdall_transport::GpioTransport;
use heimdall_transport::bitbang_jtag::{BitbangJtagTransport, BitbangPins};
use heimdall_transport::mock::MockTransport;
use std::collections::VecDeque;

fn pins() -> BitbangPins {
    BitbangPins {
        tck: 0,
        tms: 1,
        tdi: 2,
        tdo: 3,
    }
}

#[tokio::test]
async fn observe_reads_configured_output_pads() {
    let backend = MockTransport::new();
    let jtag = BitbangJtagTransport::new(backend, pins())
        .with_clock_delay(std::time::Duration::from_nanos(1));

    // Drive a separate MockTransport as the GPIO backend. Pre-load its
    // tdo_in queue with values that observe will pop via read().
    // observe walks outputs in pinmap insertion order; queue values follow.
    let mut gpio = MockTransport::new();
    // For three output pads we want values: true, false, true (in pad order).
    gpio.tdo_in = VecDeque::from(vec![true, false, true]);
    gpio.open_for_test();
    let gpio_box: Box<dyn GpioTransport> = Box::new(gpio);

    let map = IoPinmap::new()
        .with(PadEntry {
            direction: PadDirection::Out,
            fpga_pad: 0,
            gpio_line: 10,
        })
        .with(PadEntry {
            direction: PadDirection::Out,
            fpga_pad: 1,
            gpio_line: 11,
        })
        .with(PadEntry {
            direction: PadDirection::Out,
            fpga_pad: 2,
            gpio_line: 12,
        });

    let mut driver = AegisFpgaDriver::new(DutKind::AegisLuna1, jtag).with_pinmap(gpio_box, map);

    let mut dut = Dut::new(DutId::new("luna1-1"), DutKind::AegisLuna1);
    let state = driver.observe(&mut dut).await.expect("observe");
    assert_eq!(state.fields.get("io_0"), Some(&ValueRepr::Bool(true)));
    assert_eq!(state.fields.get("io_1"), Some(&ValueRepr::Bool(false)));
    assert_eq!(state.fields.get("io_2"), Some(&ValueRepr::Bool(true)));
}

#[tokio::test]
async fn observe_without_pinmap_returns_empty() {
    let backend = MockTransport::new();
    let jtag = BitbangJtagTransport::new(backend, pins())
        .with_clock_delay(std::time::Duration::from_nanos(1));
    let mut driver = AegisFpgaDriver::new(DutKind::AegisLuna1, jtag);
    let mut dut = Dut::new(DutId::new("luna1-1"), DutKind::AegisLuna1);
    let state = driver.observe(&mut dut).await.unwrap();
    assert!(state.fields.is_empty());
}
