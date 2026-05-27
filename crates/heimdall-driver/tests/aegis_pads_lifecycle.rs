//! Verifies AegisFpgaDriver opens its GPIO transport during prepare and
//! closes it during release, without the caller having to pre-open.

#![cfg(feature = "aegis")]

use heimdall_core::{DutId, DutKind, StepBudget, Stimulus};
use heimdall_driver::aegis::AegisFpgaDriver;
use heimdall_driver::aegis::pinmap::{IoPinmap, PadDirection, PadEntry};
use heimdall_driver::{Dut, TestDriver};
use heimdall_transport::GpioTransport;
use heimdall_transport::bitbang_jtag::{BitbangJtagTransport, BitbangPins};
use heimdall_transport::mock::MockTransport;
use std::collections::BTreeMap;

fn pins() -> BitbangPins {
    BitbangPins {
        tck: 0,
        tms: 1,
        tdi: 2,
        tdo: 3,
    }
}

#[tokio::test]
async fn prepare_opens_gpio_and_release_closes() {
    let backend = MockTransport::new();
    let jtag = BitbangJtagTransport::new(backend, pins())
        .with_clock_delay(std::time::Duration::from_nanos(1));

    // Construct GPIO mock UNOPENED. The bug we're guarding against would
    // surface here: any set() / read() without prepare() opening first
    // would error with NotOpen.
    let gpio_box: Box<dyn GpioTransport> = Box::new(MockTransport::new());

    let map = IoPinmap::new()
        .with(PadEntry {
            direction: PadDirection::In,
            fpga_pad: 0,
            gpio_line: 10,
        })
        .with(PadEntry {
            direction: PadDirection::Out,
            fpga_pad: 2,
            gpio_line: 12,
        });

    let mut driver = AegisFpgaDriver::new(DutKind::AegisLuna1, jtag).with_pinmap(gpio_box, map);

    let mut dut = Dut::new(DutId::new("luna1-1"), DutKind::AegisLuna1);

    // prepare should open both jtag AND gpio.
    driver.prepare(&mut dut).await.expect("prepare opens gpio");

    // run should drive io_0 via gpio.set without NotOpen errors.
    let mut inputs = BTreeMap::new();
    inputs.insert("io_0".to_string(), true);
    let stim = Stimulus {
        budget: StepBudget::cycles(1),
        inputs,
    };
    let _obs = driver.run(&mut dut, &stim).await.expect("run drives gpio");

    // observe reads io_2 via gpio.read.
    let _state = driver.observe(&mut dut).await.expect("observe reads gpio");

    // release closes both.
    driver.release(&mut dut).await.expect("release closes gpio");
}
