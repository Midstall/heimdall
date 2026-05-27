use heimdall_transport::mock::MockTransport;
use heimdall_transport::{JtagOps, ResetTarget, SerialOps, Transport, TransportError};
use std::time::Duration;

#[tokio::test]
async fn open_close_resets() {
    let mut t = MockTransport::new();
    t.open().await.unwrap();
    t.reset(ResetTarget::System).await.unwrap();
    t.close().await.unwrap();
    assert_eq!(t.resets, vec![ResetTarget::System]);
}

#[tokio::test]
async fn serial_roundtrip() {
    let mut t = MockTransport::new().with_serial_in(*b"hello\n");
    t.open().await.unwrap();
    t.write_all(b"go").await.unwrap();
    let line = t
        .read_until(b'\n', Duration::from_millis(10))
        .await
        .unwrap();
    assert_eq!(line, b"hello\n");
    assert_eq!(t.serial_out, b"go");
}

#[tokio::test]
async fn ops_fail_when_closed() {
    let mut t = MockTransport::new();
    let err = t.write_all(b"x").await.unwrap_err();
    assert!(matches!(err, TransportError::NotOpen));
}

#[tokio::test]
async fn idcode_chain_returned() {
    let mut t = MockTransport::new().with_idcode_chain([0xdead_beef]);
    t.open().await.unwrap();
    assert_eq!(t.scan_idcode().await.unwrap(), vec![0xdead_beef]);
}
