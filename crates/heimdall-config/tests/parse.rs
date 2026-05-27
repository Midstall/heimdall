use heimdall_config::load_from_path;

#[test]
fn parses_example() {
    let cfg = load_from_path("testdata/example.toml").unwrap();
    assert_eq!(cfg.host.name, "rig-01");
    assert_eq!(cfg.duts.len(), 1);
    assert_eq!(cfg.duts[0].id, "river-rc1-nano-1");
    assert_eq!(cfg.transport.uart.len(), 1);
    assert_eq!(cfg.transport.jtag.len(), 1);
    assert!(cfg.golden.river.is_some());
}

#[test]
fn missing_file_is_io_error() {
    let err = load_from_path("testdata/does-not-exist.toml").unwrap_err();
    assert!(matches!(err, heimdall_config::ConfigError::Io { .. }));
}
