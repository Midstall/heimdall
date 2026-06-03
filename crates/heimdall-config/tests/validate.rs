use heimdall_config::{ConfigError, load_from_path, validate::validate};

#[test]
fn example_validates() {
    let cfg = load_from_path("testdata/example.toml").unwrap();
    validate(&cfg).unwrap();
}

#[test]
fn dut_timeouts_block_parses_and_defaults_apply() {
    let cfg = load_from_path("testdata/timeouts.toml").unwrap();
    let dut = &cfg.duts[0];
    assert_eq!(dut.timeouts.openocd_startup_ms, 180_000);
    assert_eq!(dut.timeouts.openocd_rpc_ms, 120_000);
    assert_eq!(dut.timeouts.lease_secs, 600);
    assert_eq!(dut.timeouts.wait_halt_max_ms, 180_000);

    // The example.toml has no [dut.timeouts] block so we get the
    // silicon-friendly defaults.
    let plain = load_from_path("testdata/example.toml").unwrap();
    let pd = plain.duts[0].timeouts;
    assert_eq!(pd.openocd_startup_ms, 10_000);
    assert_eq!(pd.openocd_rpc_ms, 5_000);
    assert_eq!(pd.lease_secs, 60);
    assert_eq!(pd.wait_halt_max_ms, 30_000);
}

#[test]
fn duplicate_transport_id_rejected() {
    let cfg = load_from_path("testdata/duplicate_transport.toml").unwrap();
    let err = validate(&cfg).unwrap_err();
    assert!(matches!(err, ConfigError::DuplicateTransportId(_)));
}

#[test]
fn missing_transport_rejected() {
    let cfg = load_from_path("testdata/missing_transport.toml").unwrap();
    let err = validate(&cfg).unwrap_err();
    assert!(matches!(err, ConfigError::UnknownTransportRef { .. }));
}

use heimdall_config::{GpioDriver, GpioTransportCfg, PadDirection, PadMapEntry};

fn base_cfg_with_dut() -> heimdall_config::ConfigFile {
    heimdall_config::ConfigFile {
        host: heimdall_config::HostCfg {
            name: "rig".into(),
            bind: "127.0.0.1:7777".parse().unwrap(),
        },
        duts: vec![heimdall_config::DutCfg {
            id: "dut1".into(),
            kind: heimdall_core::DutKind::AegisLuna1,
            chip_serial: None,
            transports: vec!["gpio.host".into()],
            expect_idcode: None,
            bringup: None,
            netlist: None,
            spice_watches: vec![],
            timeouts: Default::default(),
            isa: None,
        }],
        transport: heimdall_config::TransportSection {
            gpio: vec![GpioTransportCfg {
                id: "gpio.host".into(),
                driver: GpioDriver::Mock,
                device: None,
            }],
            ..Default::default()
        },
        golden: heimdall_config::GoldenCfg {
            aegis: Some(heimdall_config::GoldenBackendCfg::Mock),
            river: None,
        },
        tools: heimdall_config::ToolsCfg::default(),
        pad_maps: vec![],
    }
}

#[test]
fn pad_map_unknown_dut_rejected() {
    let mut cfg = base_cfg_with_dut();
    cfg.pad_maps = vec![PadMapEntry {
        dut: "no-such-dut".into(),
        direction: PadDirection::In,
        fpga_pad: 0,
        gpio_line: 5,
        gpio_transport: "gpio.host".into(),
    }];
    let err = heimdall_config::validate::validate(&cfg).unwrap_err();
    assert!(matches!(
        err,
        heimdall_config::ConfigError::PadMapUnknownDut(_)
    ));
}

#[test]
fn pad_map_unknown_gpio_transport_rejected() {
    let mut cfg = base_cfg_with_dut();
    cfg.pad_maps = vec![PadMapEntry {
        dut: "dut1".into(),
        direction: PadDirection::In,
        fpga_pad: 0,
        gpio_line: 5,
        gpio_transport: "gpio.nowhere".into(),
    }];
    let err = heimdall_config::validate::validate(&cfg).unwrap_err();
    assert!(matches!(
        err,
        heimdall_config::ConfigError::PadMapUnknownGpioTransport(_)
    ));
}

#[test]
fn pad_map_duplicate_rejected() {
    let mut cfg = base_cfg_with_dut();
    cfg.pad_maps = vec![
        PadMapEntry {
            dut: "dut1".into(),
            direction: PadDirection::In,
            fpga_pad: 0,
            gpio_line: 5,
            gpio_transport: "gpio.host".into(),
        },
        PadMapEntry {
            dut: "dut1".into(),
            direction: PadDirection::In,
            fpga_pad: 0,
            gpio_line: 6,
            gpio_transport: "gpio.host".into(),
        },
    ];
    let err = heimdall_config::validate::validate(&cfg).unwrap_err();
    assert!(matches!(
        err,
        heimdall_config::ConfigError::DuplicatePadMap { .. }
    ));
}

#[test]
fn pad_map_valid_passes() {
    let mut cfg = base_cfg_with_dut();
    cfg.pad_maps = vec![
        PadMapEntry {
            dut: "dut1".into(),
            direction: PadDirection::In,
            fpga_pad: 0,
            gpio_line: 5,
            gpio_transport: "gpio.host".into(),
        },
        PadMapEntry {
            dut: "dut1".into(),
            direction: PadDirection::Out,
            fpga_pad: 2,
            gpio_line: 7,
            gpio_transport: "gpio.host".into(),
        },
    ];
    heimdall_config::validate::validate(&cfg).unwrap();
}

#[test]
fn duplicate_gpio_transport_id_rejected() {
    let mut cfg = base_cfg_with_dut();
    cfg.transport.gpio.push(GpioTransportCfg {
        id: "gpio.host".into(),
        driver: GpioDriver::Mock,
        device: None,
    });
    let err = heimdall_config::validate::validate(&cfg).unwrap_err();
    assert!(matches!(
        err,
        heimdall_config::ConfigError::DuplicateGpioTransportId(_)
    ));
}
