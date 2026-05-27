//! Hardware-in-the-loop test for RiverCpuDriver.
//!
//! Skipped unless HEIMDALL_HIL=1 AND a real OpenOCD + River target are
//! reachable at the addresses below. Not a CI gate.

#![cfg(feature = "hil")]

use std::env;
use std::net::SocketAddr;

use heimdall_core::{
    Artifact, ArtifactKind, DutId, DutKind, State, StepBudget, ValueRepr, Verdict,
};
use heimdall_driver::{Dut, TestDriver, river::RiverCpuDriver};
use heimdall_golden::{GoldenModel, spike::SpikeOneShot};
use heimdall_tools::{ToolChain, clang_asm::ClangAsm};
use heimdall_transport::openocd::OpenOcdJtagTransport;
use std::sync::Arc;

fn hil_or_skip() {
    if env::var("HEIMDALL_HIL").as_deref() != Ok("1") {
        eprintln!("skipping; set HEIMDALL_HIL=1 to run");
        std::process::exit(0);
    }
}

#[tokio::test]
#[ignore]
async fn river_hello_diff_against_spike() {
    hil_or_skip();

    let endpoint: SocketAddr = "127.0.0.1:6666".parse().unwrap();
    let jtag = OpenOcdJtagTransport::new(endpoint);
    let mut driver = RiverCpuDriver::new(DutKind::RiverRc1Nano, jtag);

    let mut golden = SpikeOneShot::new("spike", DutKind::RiverRc1Nano);

    let tools = ToolChain::new().with(Arc::new(ClangAsm::river_rv64()));

    let asm = std::fs::read("../../testdata/river-hello/hello.S").unwrap();
    let input = Artifact::new(ArtifactKind::Asm, asm);

    let mut dut = Dut::new(DutId::new("river-rc1-nano-1"), DutKind::RiverRc1Nano);

    driver.prepare(&mut dut).await.expect("prepare");
    let image = driver.compile(&input, &tools).await.expect("compile");
    golden.reset().await.unwrap();
    golden.load(&image).await.unwrap();
    driver.load(&mut dut, &image).await.expect("load");

    let _ = driver
        .run(
            &mut dut,
            &heimdall_core::Stimulus::new(StepBudget::cycles(10_000)),
        )
        .await
        .expect("run");
    let _ = golden.step(StepBudget::cycles(10_000)).await.unwrap();

    let dut_state = driver.observe(&mut dut).await.unwrap();
    let golden_state = golden.observe().await.unwrap();

    let v = driver.diff(&dut_state, &golden_state).await;
    assert!(matches!(v, Verdict::Pass), "verdict was: {:?}", v);

    // Also assert the test's hard expectation.
    let expected = State::new().with("x10", ValueRepr::U64(0x42));
    let v2 = driver.diff(&dut_state, &expected).await;
    assert!(matches!(v2, Verdict::Pass), "vs expected: {:?}", v2);

    driver.release(&mut dut).await.unwrap();
}
