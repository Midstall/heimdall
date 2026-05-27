//! Integration tests for the river DebugModule, driving real RPC against
//! MockOpenOcdServer. Verifies command formatting and response parsing.

#![cfg(feature = "river")]

mod common;

use std::time::Duration;

use common::mock_openocd::MockOpenOcdServer;
use heimdall_driver::river::debug_module::DebugModule;
use heimdall_transport::Transport;
use heimdall_transport::TransportError;
use heimdall_transport::openocd::OpenOcdJtagTransport;

async fn open_transport(server: &common::mock_openocd::RunningServer) -> OpenOcdJtagTransport {
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
