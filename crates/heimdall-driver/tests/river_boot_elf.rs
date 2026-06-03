//! Integration test: RiverCpuDriver::load parses an ELF and writes each
//! PT_LOAD segment to its physical address; ::run seeds dpc with the entry
//! before resuming. Verified against MockOpenOcdServer by inspecting the
//! commands the driver issued.

#![cfg(feature = "river")]

use std::time::Duration;

use heimdall_core::{Artifact, ArtifactKind, DutId, DutKind, StepBudget, Stimulus};
use heimdall_driver::river::RiverCpuDriver;
use heimdall_driver::{Dut, TestDriver};
use heimdall_mock_openocd::MockOpenOcdServer;
use heimdall_transport::openocd::OpenOcdJtagTransport;

/// Construct a minimal RV64 ELF with one PT_LOAD segment of 4 bytes at
/// paddr=0x10000, entry=0x100b0. Mirrors the hand-builder in the elf_loader
/// unit tests so the wire-shape stays consistent between layers.
fn rv64_elf() -> Vec<u8> {
    let mut buf = vec![0u8; 0x40 + 0x38 + 4];
    buf[0..4].copy_from_slice(b"\x7fELF");
    buf[4] = 2;
    buf[5] = 1;
    buf[6] = 1;
    buf[16..18].copy_from_slice(&2u16.to_le_bytes());
    buf[18..20].copy_from_slice(&243u16.to_le_bytes());
    buf[20..24].copy_from_slice(&1u32.to_le_bytes());
    buf[24..32].copy_from_slice(&0x100b0u64.to_le_bytes());
    buf[32..40].copy_from_slice(&0x40u64.to_le_bytes());
    buf[52..54].copy_from_slice(&0x40u16.to_le_bytes());
    buf[54..56].copy_from_slice(&0x38u16.to_le_bytes());
    buf[56..58].copy_from_slice(&1u16.to_le_bytes());

    let ph = 0x40;
    buf[ph..ph + 4].copy_from_slice(&1u32.to_le_bytes());
    buf[ph + 4..ph + 8].copy_from_slice(&5u32.to_le_bytes());
    buf[ph + 8..ph + 16].copy_from_slice(&0x78u64.to_le_bytes());
    buf[ph + 16..ph + 24].copy_from_slice(&0x10000u64.to_le_bytes());
    buf[ph + 24..ph + 32].copy_from_slice(&0x10000u64.to_le_bytes());
    buf[ph + 32..ph + 40].copy_from_slice(&4u64.to_le_bytes());
    buf[ph + 40..ph + 48].copy_from_slice(&4u64.to_le_bytes());
    buf[ph + 48..ph + 56].copy_from_slice(&0x1000u64.to_le_bytes());
    buf[0x78..0x7c].copy_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD]);
    buf
}

#[tokio::test]
async fn load_writes_segment_to_paddr_and_run_seeds_dpc() {
    let server = MockOpenOcdServer::new()
        .respond("halt", "")
        .respond("resume", "")
        .respond("wait_halt 1000", "")
        .respond("reg dpc 0x100b0", "")
        .respond_prefix("load_image", "loaded 4 bytes in 0.000s (4 KiB/s)")
        // DMI-bypass observe reads: abstractcs (poll), data0 (lo), data1 (hi).
        // abstractcs=0 -> busy=0, cmderr=0 on first poll. data0/data1=0 -> regs
        // all read 0. The command write at 0x17 can be any value; mock falls
        // back to empty for unmatched `riscv dmi_write 0x17 ...` commands.
        .respond("riscv dmi_read 0x16", "0x00000000")
        .respond("riscv dmi_read 0x4", "0x00000000")
        .respond("riscv dmi_read 0x5", "0x00000000")
        .start()
        .await;

    let mut transport = OpenOcdJtagTransport::new(server.addr());
    use heimdall_transport::Transport;
    transport.open().await.expect("connect");

    let mut driver = RiverCpuDriver::new(DutKind::RiverRc1Nano, transport)
        .with_wait_halt_max(Duration::from_secs(1));
    let mut dut = Dut::new(DutId::new("river-1"), DutKind::RiverRc1Nano);

    let image = Artifact::new(ArtifactKind::ElfRiscv, rv64_elf());
    driver.load(&mut dut, &image).await.expect("load");
    driver
        .run(&mut dut, &Stimulus::new(StepBudget::cycles(1000)))
        .await
        .expect("run");

    let received = server.received().await;

    // load_image issued at the PT_LOAD's paddr (0x10000), not the legacy
    // hardcoded 0x80000000.
    let load_cmd = received
        .iter()
        .find(|c| c.starts_with("load_image"))
        .expect("load_image command must be issued");
    assert!(
        load_cmd.contains(" 0x10000 bin"),
        "segment must be written at p_paddr=0x10000; got: {load_cmd}"
    );
    assert!(
        !load_cmd.contains("0x80000000"),
        "load must NOT use the old hardcoded 0x80000000; got: {load_cmd}"
    );

    // dpc seeded with the ELF entry (0x100b0) BEFORE resume.
    let dpc_idx = received
        .iter()
        .position(|c| c == "reg dpc 0x100b0")
        .expect("dpc must be written");
    let resume_idx = received
        .iter()
        .position(|c| c == "resume")
        .expect("resume must be issued");
    assert!(
        dpc_idx < resume_idx,
        "dpc must be set BEFORE resume; got dpc@{dpc_idx} resume@{resume_idx}"
    );

    server.shutdown().await;
}

/// Regression for the openocd 0.12.0 stale-register-cache surprise:
/// observe must read registers via DMI abstract commands (`riscv
/// dmi_write 0x17 <access-register cmd>` followed by `riscv dmi_read
/// 0x16/0x04/0x05`) rather than the cache-mediated `reg <name>`.
/// OpenOCD 0.12.0 doesn't invalidate its reg cache on the core's
/// self-halt via ebreak, so `reg <name>` would return stale zeros
/// without ever issuing an access-register on the wire. Pinning the
/// DMI ordering here so future refactors don't quietly fall back to
/// `reg <name>`.
#[tokio::test]
async fn observe_reads_registers_via_dmi_abstract_commands() {
    let server = MockOpenOcdServer::new()
        .respond("halt", "")
        .respond("resume", "")
        .respond("wait_halt 1000", "")
        .respond("reg dpc 0x100b0", "")
        .respond_prefix("load_image", "loaded 4 bytes in 0.000s (4 KiB/s)")
        // abstractcs poll: busy=0, cmderr=0 on first read.
        .respond("riscv dmi_read 0x16", "0x00000000")
        // data0/data1 reads. Values irrelevant for ordering; the
        // test only asserts the DMI shape made it onto the wire.
        .respond("riscv dmi_read 0x4", "0x00000000")
        .respond("riscv dmi_read 0x5", "0x00000000")
        .start()
        .await;

    let mut transport = OpenOcdJtagTransport::new(server.addr());
    use heimdall_transport::Transport;
    transport.open().await.expect("connect");

    let mut driver = RiverCpuDriver::new(DutKind::RiverRc1Nano, transport)
        .with_wait_halt_max(Duration::from_secs(1));
    let mut dut = Dut::new(DutId::new("river-1"), DutKind::RiverRc1Nano);

    let image = Artifact::new(ArtifactKind::ElfRiscv, rv64_elf());
    driver.load(&mut dut, &image).await.expect("load");
    driver
        .run(&mut dut, &Stimulus::new(StepBudget::cycles(1000)))
        .await
        .expect("run");

    let received = server.received().await;

    // The cache-mediated `reg <name>` form is the regression mode.
    // Permitted exceptions: the dpc-write before resume (`reg dpc <hex>`)
    // and `reg pc` is NOT allowed; pc must come from `dpc` via DMI.
    let cache_reads: Vec<&String> = received
        .iter()
        .filter(|c| c.starts_with("reg ") && !c.starts_with("reg dpc "))
        .collect();
    assert!(
        cache_reads.is_empty(),
        "observe must NOT fall back to OpenOCD's cache-mediated `reg <name>`; got: {cache_reads:?}",
    );

    // The first DMI write to 0x17 is the access-register command for x1
    // (regno = 0x1001). All 31 GPR reads + the dpc read produce 32 such
    // command writes, each followed by an abstractcs poll and two data
    // reads. Assert at least one access-register command write went out
    // and the first data0/data1 reads come after the run's wait_halt.
    let run_wait_idx = received
        .iter()
        .position(|c| c == "wait_halt 1000")
        .expect("run's wait_halt must be issued");
    // x1 access-register command: aarsize=3, transfer=1, regno=0x1001
    // -> (3 << 20) | (1 << 17) | 0x1001 = 0x00321001.
    let first_cmd_idx = received
        .iter()
        .position(|c| c == "riscv dmi_write 0x17 0x321001")
        .expect("observe must issue an access-register write for x1 (regno=0x1001)");
    let first_data0_idx = received
        .iter()
        .position(|c| c == "riscv dmi_read 0x4")
        .expect("observe must read data0 (DMI 0x04) for the GPR low half");
    assert!(
        run_wait_idx < first_cmd_idx,
        "DMI access-register must come AFTER the run's wait_halt; \
         got run_wait@{run_wait_idx} cmd@{first_cmd_idx}",
    );
    assert!(
        first_cmd_idx < first_data0_idx,
        "command write must precede data0 read; got cmd@{first_cmd_idx} data0@{first_data0_idx}",
    );

    // The last access-register write reads dpc (CSR regno 0x7b1) for the
    // PC observation: aarsize=3 << 20 | transfer=1 << 17 | 0x07b1
    // = 0x300000 | 0x020000 | 0x0007b1 = 0x3207b1.
    assert!(
        received
            .iter()
            .any(|c| c == "riscv dmi_write 0x17 0x3207b1"),
        "observe must issue an access-register write for dpc (regno=0x7b1) to read pc",
    );

    server.shutdown().await;
}

#[tokio::test]
async fn run_without_prior_load_errors() {
    let server = MockOpenOcdServer::new().start().await;
    let mut transport = OpenOcdJtagTransport::new(server.addr());
    use heimdall_transport::Transport;
    transport.open().await.expect("connect");
    let mut driver = RiverCpuDriver::new(DutKind::RiverRc1Nano, transport);
    let mut dut = Dut::new(DutId::new("river-1"), DutKind::RiverRc1Nano);
    let err = driver
        .run(&mut dut, &Stimulus::new(StepBudget::cycles(1000)))
        .await
        .expect_err("run before load must fail");
    assert!(
        format!("{err}").contains("pending_entry unset"),
        "expected pending_entry diagnostic; got `{err}`"
    );
    server.shutdown().await;
}
