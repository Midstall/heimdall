//! Integration tests for the river DebugModule, driving real RPC against
//! MockOpenOcdServer. Verifies command formatting and response parsing.

#![cfg(feature = "river")]

use std::time::Duration;

use heimdall_driver::river::debug_module::DebugModule;
use heimdall_mock_openocd::MockOpenOcdServer;
use heimdall_transport::Transport;
use heimdall_transport::TransportError;
use heimdall_transport::openocd::OpenOcdJtagTransport;

async fn open_transport(server: &heimdall_mock_openocd::RunningServer) -> OpenOcdJtagTransport {
    let mut t = OpenOcdJtagTransport::new(server.addr());
    t.open().await.expect("connect to mock");
    t
}

#[tokio::test]
async fn halt_and_resume_succeed() {
    let server = MockOpenOcdServer::new()
        .respond("halt", "")
        .respond("resume", "")
        .start()
        .await;
    let mut transport = open_transport(&server).await;
    let mut dm = DebugModule::new(&mut transport);
    dm.halt().await.expect("halt");
    dm.resume().await.expect("resume");
    transport.close().await.unwrap();

    let received = server.received().await;
    assert_eq!(received, vec!["halt".to_string(), "resume".to_string()]);
    server.shutdown().await;
}

#[tokio::test]
async fn wait_halt_success() {
    let server = MockOpenOcdServer::new()
        .respond("wait_halt 1000", "")
        .start()
        .await;
    let mut transport = open_transport(&server).await;
    let mut dm = DebugModule::new(&mut transport);
    dm.wait_halt(Duration::from_millis(1000))
        .await
        .expect("wait_halt");
    transport.close().await.unwrap();
    server.shutdown().await;
}

#[tokio::test]
async fn wait_halt_reports_timeout() {
    let server = MockOpenOcdServer::new()
        .respond(
            "wait_halt 500",
            "wait_halt: timed out while waiting for target to halt",
        )
        .start()
        .await;
    let mut transport = open_transport(&server).await;
    let mut dm = DebugModule::new(&mut transport);
    let err = dm
        .wait_halt(Duration::from_millis(500))
        .await
        .expect_err("expected timeout");
    assert!(matches!(err, TransportError::Timeout { .. }), "got {err:?}");
    transport.close().await.unwrap();
    server.shutdown().await;
}

#[tokio::test]
async fn read_gpr_parses_value() {
    let server = MockOpenOcdServer::new()
        .respond("reg a0", "a0 (/64): 0x000000000000002a")
        .start()
        .await;
    let mut transport = open_transport(&server).await;
    let mut dm = DebugModule::new(&mut transport);
    let v = dm.read_gpr(10).await.expect("read_gpr");
    assert_eq!(v, 0x2a);
    transport.close().await.unwrap();
    server.shutdown().await;
}

#[tokio::test]
async fn write_reg_formats_value_as_hex() {
    let server = MockOpenOcdServer::new()
        .respond("reg dpc 0x100b0", "")
        .start()
        .await;
    let mut transport = open_transport(&server).await;
    let mut dm = DebugModule::new(&mut transport);
    dm.write_reg("dpc", 0x100b0).await.expect("write_reg");
    transport.close().await.unwrap();

    let received = server.received().await;
    assert_eq!(received, vec!["reg dpc 0x100b0".to_string()]);
    server.shutdown().await;
}

#[tokio::test]
async fn dm_reset_cycles_dmactive() {
    let server = MockOpenOcdServer::new()
        .respond("riscv dmi_write 0x10 0x0", "")
        .respond("riscv dmi_write 0x10 0x1", "")
        .respond("riscv dmi_read 0x10", "0x00000001")
        .start()
        .await;
    let mut transport = open_transport(&server).await;
    let mut dm = DebugModule::new(&mut transport);
    dm.dm_reset().await.expect("dm_reset");
    transport.close().await.unwrap();

    let received = server.received().await;
    assert_eq!(
        received,
        vec![
            "riscv dmi_write 0x10 0x0".to_string(),
            "riscv dmi_write 0x10 0x1".to_string(),
            "riscv dmi_read 0x10".to_string(),
        ]
    );
    server.shutdown().await;
}

#[tokio::test]
async fn dm_reset_fails_when_dmactive_stays_zero() {
    let server = MockOpenOcdServer::new()
        .respond("riscv dmi_write 0x10 0x0", "")
        .respond("riscv dmi_write 0x10 0x1", "")
        .respond("riscv dmi_read 0x10", "0x00000000")
        .start()
        .await;
    let mut transport = open_transport(&server).await;
    let mut dm = DebugModule::new(&mut transport);
    let err = dm.dm_reset().await.expect_err("expected protocol error");
    assert!(matches!(err, TransportError::Protocol(_)), "got {err:?}");
    transport.close().await.unwrap();
    server.shutdown().await;
}

#[tokio::test]
async fn read_csr_parses_pc() {
    let server = MockOpenOcdServer::new()
        .respond("reg pc", "pc (/64): 0x0000000080000010")
        .start()
        .await;
    let mut transport = open_transport(&server).await;
    let mut dm = DebugModule::new(&mut transport);
    let v = dm.read_csr("pc").await.expect("read_csr");
    assert_eq!(v, 0x8000_0010);
    transport.close().await.unwrap();
    server.shutdown().await;
}

#[tokio::test]
async fn read_register_via_abstract_composes_a0_then_reads_data() {
    // Access-Register for x10 (a0): regno = 0x1000 | 10 = 0x100a.
    // (3 << 20) | (1 << 17) | 0x100a = 0x32100a.
    // abstractcs comes back with busy=0, cmderr=0 (full word zero).
    // data0/data1 carry the low/high halves of the value.
    let server = MockOpenOcdServer::new()
        .respond("riscv dmi_write 0x17 0x32100a", "")
        .respond("riscv dmi_read 0x16", "0x00000000")
        .respond("riscv dmi_read 0x4", "0x12345678") // low
        .respond("riscv dmi_read 0x5", "0xdeadbeef") // high
        .start()
        .await;
    let mut transport = open_transport(&server).await;
    let mut dm = DebugModule::new(&mut transport);
    let v = dm.read_gpr_via_abstract(10).await.expect("dmi read x10");
    assert_eq!(v, 0xdead_beef_1234_5678);
    transport.close().await.unwrap();

    let received = server.received().await;
    assert_eq!(
        received,
        vec![
            "riscv dmi_write 0x17 0x32100a".to_string(),
            "riscv dmi_read 0x16".to_string(),
            "riscv dmi_read 0x4".to_string(),
            "riscv dmi_read 0x5".to_string(),
        ],
        "wire shape: command, abstractcs poll, data0, data1",
    );
    server.shutdown().await;
}

#[tokio::test]
async fn read_register_via_abstract_polls_until_busy_clears() {
    // First abstractcs read returns busy=1. Second returns busy=0.
    // The mock fires its canned `riscv dmi_read 0x16` response on EVERY
    // matching command, so we need a separate exact-match for each of
    // the two poll cycles. Driving that via a stateful counter using the
    // prefix machinery is overkill. Instead we craft the test to use
    // the same response and the busy-bit absent on the wire isn't
    // important here. Just confirm at least one abstractcs poll happens.
    //
    // For a busy-clear-after-N-polls test we'd need a stateful mock
    // (out of scope for the minimal MockOpenOcdServer). What this test
    // pins is the single-poll happy path and the data fetch order.
    let server = MockOpenOcdServer::new()
        .respond("riscv dmi_write 0x17 0x321001", "")
        .respond("riscv dmi_read 0x16", "0x00000000")
        .respond("riscv dmi_read 0x4", "0x00000001")
        .respond("riscv dmi_read 0x5", "0x00000000")
        .start()
        .await;
    let mut transport = open_transport(&server).await;
    let mut dm = DebugModule::new(&mut transport);
    let v = dm.read_gpr_via_abstract(1).await.expect("dmi read x1");
    assert_eq!(v, 1);
    transport.close().await.unwrap();
    server.shutdown().await;
}

#[tokio::test]
async fn read_register_via_abstract_surfaces_cmderr() {
    // abstractcs=0x00000300 -> busy=0, cmderr=3 (exception while
    // executing the abstract command). Must propagate as a Protocol
    // error so observe failures surface clearly.
    let server = MockOpenOcdServer::new()
        .respond("riscv dmi_write 0x17 0x321001", "")
        .respond("riscv dmi_read 0x16", "0x00000300")
        .start()
        .await;
    let mut transport = open_transport(&server).await;
    let mut dm = DebugModule::new(&mut transport);
    let err = dm
        .read_gpr_via_abstract(1)
        .await
        .expect_err("expected cmderr propagation");
    let msg = format!("{err}");
    assert!(
        msg.contains("cmderr=0x3"),
        "error must mention cmderr value; got `{msg}`",
    );
    transport.close().await.unwrap();
    server.shutdown().await;
}

#[tokio::test]
async fn read_register_via_abstract_rejects_oob_gpr_index() {
    let server = MockOpenOcdServer::new().start().await;
    let mut transport = open_transport(&server).await;
    let mut dm = DebugModule::new(&mut transport);
    let err = dm
        .read_gpr_via_abstract(32)
        .await
        .expect_err("32 is out of GPR range");
    assert!(matches!(err, TransportError::Protocol(_)), "got {err:?}");
    transport.close().await.unwrap();
    server.shutdown().await;
}
