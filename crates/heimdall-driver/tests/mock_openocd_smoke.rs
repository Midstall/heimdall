//! Smoke test that MockOpenOcdServer accepts a connection and echoes a canned response.

#![cfg(feature = "river")]

mod common;

use common::mock_openocd::MockOpenOcdServer;
use heimdall_transport::Transport;
use heimdall_transport::openocd::OpenOcdJtagTransport;

#[tokio::test]
async fn mock_server_responds_to_canned_command() {
    let server = MockOpenOcdServer::new()
        .respond("halt", "")
        .respond("ping", "pong")
        .start()
        .await;
    let mut transport = OpenOcdJtagTransport::new(server.addr());
    transport.open().await.expect("connect");

    let pong = transport.rpc("ping").await.expect("rpc");
    assert_eq!(pong, "pong");

    let halt_resp = transport.rpc("halt").await.expect("rpc");
    assert_eq!(halt_resp, "");

    transport.close().await.expect("close");
    let received = server.received().await;
    assert_eq!(received, vec!["ping".to_string(), "halt".to_string()]);
    server.shutdown().await;
}
