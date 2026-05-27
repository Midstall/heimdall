//! Unit tests for `TransportSpec::probe_connection`. Doesn't exercise the
//! HTTP layer; just probes each variant directly so the matrix is fast and
//! deterministic.

use std::net::SocketAddr;
use std::path::PathBuf;

use heimdall_daemon::{ConnectionStatus, TransportSpec};
use tempfile::TempDir;

#[tokio::test]
async fn mock_probe_is_unknown() {
    let spec = TransportSpec::Mock;
    assert_eq!(spec.probe_connection().await, ConnectionStatus::Unknown);
}

#[tokio::test]
async fn bitbang_cdev_probe_connected_when_path_exists() {
    // /dev/null exists on every supported target and is a char device.
    let spec = TransportSpec::BitbangCdev {
        device: PathBuf::from("/dev/null"),
        tck: 0,
        tms: 1,
        tdi: 2,
        tdo: 3,
        freq_hz: 1_000_000,
    };
    assert_eq!(spec.probe_connection().await, ConnectionStatus::Connected);
}

#[tokio::test]
async fn bitbang_cdev_probe_disconnected_when_missing() {
    let spec = TransportSpec::BitbangCdev {
        device: PathBuf::from("/nonexistent/gpiochip-xyz"),
        tck: 0,
        tms: 1,
        tdi: 2,
        tdo: 3,
        freq_hz: 1_000_000,
    };
    assert_eq!(
        spec.probe_connection().await,
        ConnectionStatus::Disconnected
    );
}

#[tokio::test]
async fn openocd_probe_connected_against_live_listener() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint: SocketAddr = listener.local_addr().unwrap();
    // Accept and drop one connection in the background so probe() succeeds.
    let _accept = tokio::spawn(async move {
        let _ = listener.accept().await;
    });
    let spec = TransportSpec::Openocd { endpoint };
    assert_eq!(spec.probe_connection().await, ConnectionStatus::Connected);
}

#[tokio::test]
async fn openocd_probe_disconnected_on_unbound_port() {
    // Use a reserved port range very unlikely to have anything listening.
    let endpoint: SocketAddr = "127.0.0.1:1".parse().unwrap();
    let spec = TransportSpec::Openocd { endpoint };
    let status = spec.probe_connection().await;
    assert!(
        matches!(status, ConnectionStatus::Disconnected),
        "expected disconnected, got {status:?}"
    );
}

#[tokio::test]
async fn openocd_spawned_probe_unknown_when_config_present() {
    let tmp = TempDir::new().unwrap();
    let cfg_path = tmp.path().join("openocd.cfg");
    std::fs::write(&cfg_path, b"# minimal openocd config").unwrap();
    let spec = TransportSpec::OpenocdSpawned {
        binary: PathBuf::from("/usr/bin/openocd"),
        config_file: cfg_path,
        tcl_port: 6666,
        extra_args: vec![],
    };
    // We can't verify the subprocess from the registry. Existence of the
    // config is the only signal we have, and that just clears Disconnected.
    assert_eq!(spec.probe_connection().await, ConnectionStatus::Unknown);
}

#[tokio::test]
async fn openocd_spawned_probe_disconnected_when_config_missing() {
    let spec = TransportSpec::OpenocdSpawned {
        binary: PathBuf::from("/usr/bin/openocd"),
        config_file: PathBuf::from("/nonexistent/openocd.cfg"),
        tcl_port: 6666,
        extra_args: vec![],
    };
    assert_eq!(
        spec.probe_connection().await,
        ConnectionStatus::Disconnected
    );
}

#[tokio::test]
async fn ftdi_probe_disconnected_for_implausible_vid_pid() {
    // VID 0xFFFF is reserved/invalid. Even with the `ftdi` feature on this
    // should never match a real device.
    let spec = TransportSpec::Ftdi {
        serial: None,
        vid: Some(0xFFFF),
        pid: Some(0xFFFF),
        interface: Some(0),
    };
    let status = spec.probe_connection().await;
    // Without the `ftdi` daemon feature we return Unknown. With it on we
    // return Disconnected. Both are valid outcomes for this matrix entry.
    assert!(
        matches!(
            status,
            ConnectionStatus::Disconnected | ConnectionStatus::Unknown
        ),
        "expected disconnected or unknown, got {status:?}"
    );
}
