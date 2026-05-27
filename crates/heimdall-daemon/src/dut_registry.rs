//! DUT registry built from `heimdall.toml`. Each `DutRecord` knows what
//! kind of DUT it is, its chip serial (if known), and a `TransportSpec`
//! that describes how to construct a fresh transport when a job runs.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::PathBuf;

use heimdall_config::ConfigFile;
use heimdall_config::JtagDriver;
use heimdall_core::{DutId, DutKind};
use serde::Serialize;

use crate::error::{DaemonError, Result};

/// What to construct when a job needs the JTAG transport for a DUT.
///
/// Constructed lazily per-job by the factory. The spec itself is plain data.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "kebab-case", tag = "driver")]
pub enum TransportSpec {
    /// Mock transport for testing. Selected when the configured JTAG driver
    /// is `mock`.
    Mock,
    /// Bit-banged JTAG over Linux GPIO character device. Selected when the
    /// configured JTAG driver is `bitbang-jtag`.
    BitbangCdev {
        device: PathBuf,
        tck: u32,
        tms: u32,
        tdi: u32,
        tdo: u32,
        freq_hz: u32,
    },
    /// OpenOCD-managed JTAG over its Tcl RPC port. Selected when the
    /// configured JTAG driver is `openocd`.
    Openocd { endpoint: SocketAddr },
    /// OpenOCD process spawned by the daemon. The daemon starts `binary` with
    /// `-f config_file` and listens on `tcl_port` for Tcl RPC. Selected when
    /// the configured JTAG driver is `openocd-spawn`.
    OpenocdSpawned {
        binary: PathBuf,
        config_file: PathBuf,
        tcl_port: u16,
        #[serde(default)]
        extra_args: Vec<String>,
    },
    /// FTDI JTAG, either via the native pure-Rust MPSSE driver (heimdall-
    /// transport's `ftdi` feature) or OpenOCD fallback. The vid/pid/interface
    /// fields come from `heimdall.toml` and the connection-status probe uses
    /// them to look the device up in the host USB enumeration.
    Ftdi {
        serial: Option<String>,
        #[serde(default)]
        vid: Option<u16>,
        #[serde(default)]
        pid: Option<u16>,
        #[serde(default)]
        interface: Option<u8>,
    },
}

/// Live-probe result for a configured DUT's transport. Computed at
/// `/duts` request time, not cached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConnectionStatus {
    /// We checked and the device is reachable (USB device present, TCP
    /// connect succeeded, GPIO chardev exists).
    Connected,
    /// We checked and the device is not reachable.
    Disconnected,
    /// We couldn't determine state (mock transport, spawned subprocess we
    /// don't own yet, optional feature disabled).
    Unknown,
}

impl TransportSpec {
    /// Best-effort liveness probe for this transport. Returns quickly and
    /// never blocks the request thread for more than a few hundred ms.
    pub async fn probe_connection(&self) -> ConnectionStatus {
        use std::time::Duration;
        match self {
            TransportSpec::Mock => ConnectionStatus::Unknown,
            TransportSpec::BitbangCdev { device, .. } => match tokio::fs::metadata(device).await {
                Ok(_) => ConnectionStatus::Connected,
                Err(_) => ConnectionStatus::Disconnected,
            },
            TransportSpec::Openocd { endpoint } => {
                let connect = tokio::net::TcpStream::connect(endpoint);
                match tokio::time::timeout(Duration::from_millis(250), connect).await {
                    Ok(Ok(_)) => ConnectionStatus::Connected,
                    Ok(Err(_)) | Err(_) => ConnectionStatus::Disconnected,
                }
            }
            // We spawn the subprocess per-job, so we can't say from the
            // registry whether it's currently up. Verify the config file
            // at least exists. Binary presence on $PATH is a doctor check.
            TransportSpec::OpenocdSpawned { config_file, .. } => {
                match tokio::fs::metadata(config_file).await {
                    Ok(_) => ConnectionStatus::Unknown,
                    Err(_) => ConnectionStatus::Disconnected,
                }
            }
            TransportSpec::Ftdi {
                serial, vid, pid, ..
            } => probe_ftdi(serial.as_deref(), *vid, *pid),
        }
    }
}

#[cfg(feature = "ftdi")]
fn probe_ftdi(serial: Option<&str>, vid: Option<u16>, pid: Option<u16>) -> ConnectionStatus {
    let matches = heimdall_transport::ftdi::enumerate_ftdi_devices(vid, pid, serial);
    if matches.is_empty() {
        ConnectionStatus::Disconnected
    } else {
        ConnectionStatus::Connected
    }
}

#[cfg(not(feature = "ftdi"))]
fn probe_ftdi(_serial: Option<&str>, _vid: Option<u16>, _pid: Option<u16>) -> ConnectionStatus {
    // Without the `ftdi` daemon feature we have no USB enumeration backend.
    ConnectionStatus::Unknown
}

/// How to construct the GPIO transport that drives FPGA pads. Pulled from
/// the `[[transport.gpio]]` entry referenced by the DUT's pad_map.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "kebab-case", tag = "driver")]
pub enum GpioSpec {
    Mock,
    LinuxCdev { device: PathBuf },
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PadDirection {
    In,
    Out,
}

impl From<heimdall_config::PadDirection> for PadDirection {
    fn from(d: heimdall_config::PadDirection) -> Self {
        match d {
            heimdall_config::PadDirection::In => Self::In,
            heimdall_config::PadDirection::Out => Self::Out,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct PadEntry {
    pub direction: PadDirection,
    pub fpga_pad: u32,
    pub gpio_line: u32,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct IoPinmap {
    pub gpio_spec: Option<GpioSpec>,
    pub entries: Vec<PadEntry>,
}

impl IoPinmap {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Resolved bringup payload: file paths from the BringupSpec have been read
/// into memory at build_registry time.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "kebab-case", tag = "type")]
pub enum BringupPayload {
    AegisVector {
        descriptor_json: String,
        #[serde(skip_serializing)]
        bitstream: Vec<u8>,
        bitstream_len: usize,
        inputs: BTreeMap<String, bool>,
        expected_outputs: BTreeMap<String, bool>,
        settle_cycles: u64,
    },
    RiverElf {
        #[serde(skip_serializing)]
        elf: Vec<u8>,
        elf_len: usize,
        cycles: u64,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct DutRecord {
    pub id: DutId,
    pub kind: DutKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chip_serial: Option<String>,
    pub jtag: TransportSpec,
    #[serde(skip_serializing_if = "IoPinmap::is_empty")]
    pub pad_map: IoPinmap,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bringup: Option<BringupPayload>,
    /// Absolute path to a SPICE netlist describing this DUT (analog/mixed-signal
    /// flows). The daemon's `/duts/:id/netlist.svg` endpoint reads this file
    /// at request time and renders it via [`heimdall_golden::render_netlist`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub netlist: Option<PathBuf>,
    /// Heimdall-pad-to-spice-node watch list used by the renderer to highlight
    /// inputs/outputs in the SVG overlay.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spice_watches: Vec<SpiceWatch>,
}

/// Daemon-side mirror of [`heimdall_config::SpiceWatchCfg`]. Lives here so
/// downstream crates that already depend on heimdall-daemon don't need a
/// transitive heimdall-config dep just to read watches.
#[derive(Debug, Clone, Serialize)]
pub struct SpiceWatch {
    pub name: String,
    pub spice_node: String,
    pub direction: PadDirection,
}

impl From<&heimdall_config::SpiceWatchCfg> for SpiceWatch {
    fn from(cfg: &heimdall_config::SpiceWatchCfg) -> Self {
        Self {
            name: cfg.name.clone(),
            spice_node: cfg.spice_node.clone(),
            direction: cfg.direction.into(),
        }
    }
}

/// How to construct the GoldenModel for a DUT family (river or aegis).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "kebab-case", tag = "backend")]
pub enum GoldenSpec {
    Mock,
    SpikeOneShot {
        binary: PathBuf,
        #[serde(default)]
        extra_args: Vec<String>,
    },
    DartRpc {
        entry: PathBuf,
    },
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct DutRegistry {
    #[serde(serialize_with = "serialize_btreemap_values")]
    duts: BTreeMap<DutId, DutRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub river_golden: Option<GoldenSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aegis_golden: Option<GoldenSpec>,
}

fn serialize_btreemap_values<S, V>(
    map: &BTreeMap<DutId, V>,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error>
where
    S: serde::Serializer,
    V: Serialize,
{
    use serde::ser::SerializeSeq;
    let mut seq = serializer.serialize_seq(Some(map.len()))?;
    for v in map.values() {
        seq.serialize_element(v)?;
    }
    seq.end()
}

impl DutRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, rec: DutRecord) {
        self.duts.insert(rec.id.clone(), rec);
    }

    pub fn lookup(&self, id: &DutId) -> Option<&DutRecord> {
        self.duts.get(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &DutRecord> {
        self.duts.values()
    }

    pub fn len(&self) -> usize {
        self.duts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.duts.is_empty()
    }

    /// Return the configured golden spec for a DUT's family, or None if no
    /// golden is configured.
    pub fn golden_for(&self, kind: DutKind) -> Option<&GoldenSpec> {
        use heimdall_core::kind::Family;
        match kind.family() {
            Family::Cpu => self.river_golden.as_ref(),
            Family::Fpga => self.aegis_golden.as_ref(),
        }
    }
}

/// Resolve every DUT in the config to a DutRecord. The first JTAG transport
/// referenced by the DUT's `transports` list is used as its JTAG spec. If no
/// JTAG transport is referenced, the DUT is rejected.
///
/// Relative paths inside `bringup` specs resolve against the current working
/// directory. Use `build_registry_with_root` to resolve relative paths against
/// the directory containing `heimdall.toml`.
pub fn build_registry(cfg: &ConfigFile) -> Result<DutRegistry> {
    build_registry_with_root(cfg, None)
}

/// Like `build_registry` but resolves relative bringup file paths against
/// `config_dir`. Absolute paths are used verbatim. Pass `None` to resolve
/// against the current working directory.
pub fn build_registry_with_root(
    cfg: &ConfigFile,
    config_dir: Option<&std::path::Path>,
) -> Result<DutRegistry> {
    // Index JTAG transports by id for quick lookup.
    let mut jtag_by_id = BTreeMap::new();
    for t in &cfg.transport.jtag {
        if jtag_by_id.insert(t.id.clone(), t.clone()).is_some() {
            return Err(DaemonError::Config(format!(
                "duplicate jtag transport id `{}`",
                t.id
            )));
        }
    }

    // Index GPIO transports by id for pad_map resolution.
    let mut gpio_by_id = BTreeMap::new();
    for g in &cfg.transport.gpio {
        if gpio_by_id.insert(g.id.clone(), g.clone()).is_some() {
            return Err(DaemonError::Config(format!(
                "duplicate gpio transport id `{}`",
                g.id
            )));
        }
    }

    // Group pad_map entries by dut id.
    let mut pads_by_dut: BTreeMap<String, Vec<&heimdall_config::PadMapEntry>> = BTreeMap::new();
    for entry in &cfg.pad_maps {
        pads_by_dut
            .entry(entry.dut.clone())
            .or_default()
            .push(entry);
    }

    let mut registry = DutRegistry::new();
    for d in &cfg.duts {
        // Find the first transport ref that matches a configured jtag id.
        let jtag_ref = d
            .transports
            .iter()
            .find(|tref| jtag_by_id.contains_key(*tref))
            .ok_or_else(|| {
                DaemonError::Config(format!(
                    "dut `{}` references no jtag transport (refs: {:?})",
                    d.id, d.transports
                ))
            })?;
        let jtag_cfg = jtag_by_id
            .get(jtag_ref)
            .expect("checked contains_key above")
            .clone();

        let spec = match jtag_cfg.driver {
            JtagDriver::Mock => TransportSpec::Mock,
            JtagDriver::BitbangJtag => {
                // Default pin layout. heimdall.toml's transport.jtag block
                // does not yet have a structured way to declare bitbang pins;
                // until it does, operators wanting non-default pins must point
                // at a separate config.
                TransportSpec::BitbangCdev {
                    device: PathBuf::from("/dev/gpiochip0"),
                    tck: 17,
                    tms: 27,
                    tdi: 22,
                    tdo: 23,
                    freq_hz: jtag_cfg.freq_hz,
                }
            }
            JtagDriver::Openocd => {
                let endpoint = jtag_cfg.openocd_endpoint.ok_or_else(|| {
                    DaemonError::Config(format!(
                        "jtag `{}` driver=openocd missing openocd_endpoint",
                        jtag_cfg.id
                    ))
                })?;
                TransportSpec::Openocd { endpoint }
            }
            JtagDriver::OpenocdSpawn => {
                let config_file = jtag_cfg.openocd_config.clone().ok_or_else(|| {
                    DaemonError::Config(format!(
                        "jtag `{}` driver=openocd-spawn missing openocd_config",
                        jtag_cfg.id
                    ))
                })?;
                let binary = jtag_cfg
                    .openocd_binary
                    .clone()
                    .unwrap_or_else(|| PathBuf::from("openocd"));
                let tcl_port = jtag_cfg
                    .openocd_endpoint
                    .map(|sa| sa.port())
                    .unwrap_or(6666);
                TransportSpec::OpenocdSpawned {
                    binary,
                    config_file,
                    tcl_port,
                    extra_args: jtag_cfg.openocd_extra_args.clone(),
                }
            }
            JtagDriver::Ftdi => TransportSpec::Ftdi {
                serial: jtag_cfg.serial.clone(),
                vid: jtag_cfg.ftdi_vid,
                pid: jtag_cfg.ftdi_pid,
                interface: jtag_cfg.ftdi_interface,
            },
        };

        // Resolve pad_map entries for this DUT.
        let mut pad_entries = Vec::new();
        let mut gpio_spec: Option<GpioSpec> = None;
        if let Some(entries) = pads_by_dut.get(&d.id) {
            for entry in entries {
                let gcfg = gpio_by_id.get(&entry.gpio_transport).ok_or_else(|| {
                    DaemonError::Config(format!(
                        "pad_map for dut `{}` references unknown gpio transport `{}`",
                        d.id, entry.gpio_transport
                    ))
                })?;
                let resolved = match gcfg.driver {
                    heimdall_config::GpioDriver::Mock => GpioSpec::Mock,
                    heimdall_config::GpioDriver::LinuxCdev => GpioSpec::LinuxCdev {
                        device: gcfg
                            .device
                            .clone()
                            .unwrap_or_else(|| std::path::PathBuf::from("/dev/gpiochip0")),
                    },
                };
                if gpio_spec.is_some()
                    && !matches!(
                        (gpio_spec.as_ref(), &resolved),
                        (Some(GpioSpec::Mock), GpioSpec::Mock)
                            | (Some(GpioSpec::LinuxCdev { .. }), GpioSpec::LinuxCdev { .. })
                    )
                {
                    return Err(DaemonError::Config(format!(
                        "pad_map for dut `{}` uses multiple gpio transports; not yet supported",
                        d.id
                    )));
                }
                gpio_spec = Some(resolved);
                pad_entries.push(PadEntry {
                    direction: entry.direction.into(),
                    fpga_pad: entry.fpga_pad,
                    gpio_line: entry.gpio_line,
                });
            }
        }

        // Resolve bringup spec (reads files into memory).
        let bringup = if let Some(spec) = &d.bringup {
            Some(resolve_bringup(spec, config_dir)?)
        } else {
            None
        };

        // Resolve netlist path: existence is checked here so the operator
        // sees the error at startup rather than at the first HTTP request.
        let netlist = if let Some(p) = &d.netlist {
            let resolved = resolve_path(p, config_dir);
            if !resolved.exists() {
                return Err(DaemonError::Config(format!(
                    "dut `{}` netlist {} does not exist",
                    d.id,
                    resolved.display()
                )));
            }
            Some(resolved)
        } else {
            None
        };

        let spice_watches: Vec<SpiceWatch> = d.spice_watches.iter().map(SpiceWatch::from).collect();

        registry.insert(DutRecord {
            id: DutId::new(d.id.clone()),
            kind: d.kind,
            chip_serial: d.chip_serial.clone(),
            jtag: spec,
            pad_map: IoPinmap {
                gpio_spec,
                entries: pad_entries,
            },
            bringup,
            netlist,
            spice_watches,
        });
    }

    registry.river_golden = cfg.golden.river.as_ref().map(spec_from_cfg);
    registry.aegis_golden = cfg.golden.aegis.as_ref().map(spec_from_cfg);

    Ok(registry)
}

fn resolve_bringup(
    spec: &heimdall_config::BringupSpec,
    config_dir: Option<&std::path::Path>,
) -> Result<BringupPayload> {
    match spec {
        heimdall_config::BringupSpec::AegisVector {
            descriptor_path,
            bitstream_path,
            inputs,
            expected_outputs,
            settle_cycles,
        } => {
            let descriptor_json = read_text(descriptor_path, config_dir)?;
            let bitstream = read_bytes(bitstream_path, config_dir)?;
            let bitstream_len = bitstream.len();
            Ok(BringupPayload::AegisVector {
                descriptor_json,
                bitstream,
                bitstream_len,
                inputs: inputs.clone(),
                expected_outputs: expected_outputs.clone(),
                settle_cycles: *settle_cycles,
            })
        }
        heimdall_config::BringupSpec::RiverElf { elf_path, cycles } => {
            let elf = read_bytes(elf_path, config_dir)?;
            let elf_len = elf.len();
            Ok(BringupPayload::RiverElf {
                elf,
                elf_len,
                cycles: *cycles,
            })
        }
    }
}

fn resolve_path(
    path: &std::path::Path,
    config_dir: Option<&std::path::Path>,
) -> std::path::PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else if let Some(dir) = config_dir {
        dir.join(path)
    } else {
        path.to_path_buf()
    }
}

fn read_text(path: &std::path::Path, config_dir: Option<&std::path::Path>) -> Result<String> {
    let resolved = resolve_path(path, config_dir);
    std::fs::read_to_string(&resolved).map_err(|e| {
        DaemonError::Config(format!(
            "bringup: could not read {} ({e})",
            resolved.display()
        ))
    })
}

fn read_bytes(path: &std::path::Path, config_dir: Option<&std::path::Path>) -> Result<Vec<u8>> {
    let resolved = resolve_path(path, config_dir);
    std::fs::read(&resolved).map_err(|e| {
        DaemonError::Config(format!(
            "bringup: could not read {} ({e})",
            resolved.display()
        ))
    })
}

fn spec_from_cfg(cfg: &heimdall_config::GoldenBackendCfg) -> GoldenSpec {
    use heimdall_config::GoldenBackendCfg;
    match cfg {
        GoldenBackendCfg::Mock => GoldenSpec::Mock,
        GoldenBackendCfg::SpikeOneShot { binary, extra_args } => GoldenSpec::SpikeOneShot {
            binary: binary.clone(),
            extra_args: extra_args.clone(),
        },
        GoldenBackendCfg::DartRpc { entry, .. } => GoldenSpec::DartRpc {
            entry: entry.clone(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dut_registry::{BringupPayload, build_registry_with_root};
    use heimdall_config::{
        ConfigFile, DutCfg, GoldenBackendCfg, GoldenCfg, HostCfg, JtagDriver, JtagTransportCfg,
        ToolsCfg, TransportSection,
    };

    fn host() -> HostCfg {
        HostCfg {
            name: "rig-01".into(),
            bind: "127.0.0.1:7777".parse().unwrap(),
        }
    }

    fn jtag_mock(id: &str) -> JtagTransportCfg {
        JtagTransportCfg {
            id: id.into(),
            driver: JtagDriver::Mock,
            serial: None,
            openocd_endpoint: None,
            openocd_config: None,
            openocd_binary: None,
            openocd_extra_args: vec![],
            freq_hz: 1_000_000,
            ftdi_vid: None,
            ftdi_pid: None,
            ftdi_interface: None,
        }
    }

    fn jtag_openocd(id: &str, endpoint: &str) -> JtagTransportCfg {
        JtagTransportCfg {
            id: id.into(),
            driver: JtagDriver::Openocd,
            serial: None,
            openocd_endpoint: Some(endpoint.parse().unwrap()),
            openocd_config: None,
            openocd_binary: None,
            openocd_extra_args: vec![],
            freq_hz: 1_000_000,
            ftdi_vid: None,
            ftdi_pid: None,
            ftdi_interface: None,
        }
    }

    fn jtag_openocd_spawn(id: &str) -> JtagTransportCfg {
        JtagTransportCfg {
            id: id.into(),
            driver: JtagDriver::OpenocdSpawn,
            serial: None,
            openocd_endpoint: Some("127.0.0.1:6666".parse().unwrap()),
            openocd_config: Some(PathBuf::from("/tmp/heimdall-test.cfg")),
            openocd_binary: Some(PathBuf::from("/usr/bin/openocd")),
            openocd_extra_args: vec!["-d3".into()],
            freq_hz: 1_000_000,
            ftdi_vid: None,
            ftdi_pid: None,
            ftdi_interface: None,
        }
    }

    fn dut(id: &str, kind: DutKind, transports: Vec<&str>) -> DutCfg {
        DutCfg {
            id: id.into(),
            kind,
            chip_serial: None,
            transports: transports.into_iter().map(String::from).collect(),
            expect_idcode: None,
            bringup: None,
            netlist: None,
            spice_watches: vec![],
        }
    }

    #[test]
    fn resolves_mock_jtag() {
        let cfg = ConfigFile {
            host: host(),
            duts: vec![dut("luna1-1", DutKind::AegisLuna1, vec!["jtag.mock"])],
            transport: TransportSection {
                jtag: vec![jtag_mock("jtag.mock")],
                ..Default::default()
            },
            golden: GoldenCfg::default(),
            tools: ToolsCfg::default(),
            pad_maps: vec![],
        };
        let reg = build_registry(&cfg).expect("registry");
        assert_eq!(reg.len(), 1);
        let r = reg.lookup(&DutId::new("luna1-1")).unwrap();
        assert!(matches!(r.jtag, TransportSpec::Mock));
        assert_eq!(r.kind, DutKind::AegisLuna1);
    }

    #[test]
    fn resolves_openocd() {
        let cfg = ConfigFile {
            host: host(),
            duts: vec![dut("luna1-1", DutKind::AegisLuna1, vec!["jtag.ocd"])],
            transport: TransportSection {
                jtag: vec![jtag_openocd("jtag.ocd", "127.0.0.1:6666")],
                ..Default::default()
            },
            golden: GoldenCfg::default(),
            tools: ToolsCfg::default(),
            pad_maps: vec![],
        };
        let reg = build_registry(&cfg).expect("registry");
        let r = reg.lookup(&DutId::new("luna1-1")).unwrap();
        match &r.jtag {
            TransportSpec::Openocd { endpoint } => {
                assert_eq!(endpoint.to_string(), "127.0.0.1:6666");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn rejects_dut_without_jtag_transport() {
        let cfg = ConfigFile {
            host: host(),
            duts: vec![dut("dut1", DutKind::AegisLuna1, vec!["uart.usb0"])],
            transport: TransportSection {
                // No jtag transports at all.
                ..Default::default()
            },
            golden: GoldenCfg::default(),
            tools: ToolsCfg::default(),
            pad_maps: vec![],
        };
        let err = build_registry(&cfg).expect_err("should reject");
        let msg = format!("{err}");
        assert!(msg.contains("no jtag transport"), "got `{msg}`");
    }

    #[test]
    fn rejects_openocd_without_endpoint() {
        let cfg = ConfigFile {
            host: host(),
            duts: vec![dut("luna1-1", DutKind::AegisLuna1, vec!["jtag.ocd"])],
            transport: TransportSection {
                jtag: vec![JtagTransportCfg {
                    id: "jtag.ocd".into(),
                    driver: JtagDriver::Openocd,
                    serial: None,
                    openocd_endpoint: None,
                    openocd_config: None,
                    openocd_binary: None,
                    openocd_extra_args: vec![],
                    freq_hz: 1_000_000,
                    ftdi_vid: None,
                    ftdi_pid: None,
                    ftdi_interface: None,
                }],
                ..Default::default()
            },
            golden: GoldenCfg {
                river: Some(GoldenBackendCfg::Mock),
                aegis: None,
            },
            tools: ToolsCfg::default(),
            pad_maps: vec![],
        };
        let err = build_registry(&cfg).expect_err("should reject");
        let msg = format!("{err}");
        assert!(msg.contains("missing openocd_endpoint"), "got `{msg}`");
    }

    #[test]
    fn rejects_duplicate_jtag_id() {
        let cfg = ConfigFile {
            host: host(),
            duts: vec![],
            transport: TransportSection {
                jtag: vec![jtag_mock("jtag.x"), jtag_mock("jtag.x")],
                ..Default::default()
            },
            golden: GoldenCfg::default(),
            tools: ToolsCfg::default(),
            pad_maps: vec![],
        };
        let err = build_registry(&cfg).expect_err("should reject");
        let msg = format!("{err}");
        assert!(msg.contains("duplicate jtag transport"), "got `{msg}`");
    }

    #[test]
    fn populates_river_golden_from_config() {
        let cfg = ConfigFile {
            host: host(),
            duts: vec![],
            transport: TransportSection::default(),
            golden: GoldenCfg {
                river: Some(GoldenBackendCfg::SpikeOneShot {
                    binary: PathBuf::from("/usr/bin/spike"),
                    extra_args: vec!["--isa=rv64gc".into()],
                }),
                aegis: None,
            },
            tools: ToolsCfg::default(),
            pad_maps: vec![],
        };
        let reg = build_registry(&cfg).expect("registry");
        match reg.river_golden.as_ref().expect("river golden") {
            GoldenSpec::SpikeOneShot { binary, extra_args } => {
                assert_eq!(binary.to_str().unwrap(), "/usr/bin/spike");
                assert_eq!(extra_args, &vec!["--isa=rv64gc".to_string()]);
            }
            other => panic!("unexpected spec {other:?}"),
        }
        assert!(reg.aegis_golden.is_none());
    }

    #[test]
    fn populates_aegis_mock_golden() {
        let cfg = ConfigFile {
            host: host(),
            duts: vec![],
            transport: TransportSection::default(),
            golden: GoldenCfg {
                river: None,
                aegis: Some(GoldenBackendCfg::Mock),
            },
            tools: ToolsCfg::default(),
            pad_maps: vec![],
        };
        let reg = build_registry(&cfg).expect("registry");
        assert!(matches!(reg.aegis_golden.as_ref(), Some(GoldenSpec::Mock)));
    }

    #[test]
    fn golden_for_routes_by_family() {
        let mut reg = DutRegistry::new();
        reg.river_golden = Some(GoldenSpec::Mock);
        reg.aegis_golden = Some(GoldenSpec::SpikeOneShot {
            binary: PathBuf::from("/x"),
            extra_args: vec![],
        });
        assert!(matches!(
            reg.golden_for(DutKind::RiverRc1Nano),
            Some(GoldenSpec::Mock)
        ));
        assert!(matches!(
            reg.golden_for(DutKind::AegisLuna1),
            Some(GoldenSpec::SpikeOneShot { .. })
        ));
    }

    #[test]
    fn resolves_pad_map_with_mock_gpio() {
        let cfg = ConfigFile {
            host: host(),
            duts: vec![dut("luna1-1", DutKind::AegisLuna1, vec!["jtag.mock"])],
            transport: TransportSection {
                jtag: vec![jtag_mock("jtag.mock")],
                gpio: vec![heimdall_config::GpioTransportCfg {
                    id: "gpio.host".into(),
                    driver: heimdall_config::GpioDriver::Mock,
                    device: None,
                }],
                ..Default::default()
            },
            golden: GoldenCfg {
                aegis: Some(GoldenBackendCfg::Mock),
                river: None,
            },
            tools: ToolsCfg::default(),
            pad_maps: vec![
                heimdall_config::PadMapEntry {
                    dut: "luna1-1".into(),
                    direction: heimdall_config::PadDirection::In,
                    fpga_pad: 0,
                    gpio_line: 5,
                    gpio_transport: "gpio.host".into(),
                },
                heimdall_config::PadMapEntry {
                    dut: "luna1-1".into(),
                    direction: heimdall_config::PadDirection::Out,
                    fpga_pad: 1,
                    gpio_line: 6,
                    gpio_transport: "gpio.host".into(),
                },
            ],
        };
        let reg = build_registry(&cfg).expect("registry");
        let r = reg.lookup(&DutId::new("luna1-1")).expect("dut");
        assert_eq!(r.pad_map.entries.len(), 2);
        assert!(matches!(r.pad_map.gpio_spec, Some(GpioSpec::Mock)));
    }

    #[test]
    fn resolves_pad_map_with_linux_cdev() {
        let cfg = ConfigFile {
            host: host(),
            duts: vec![dut("luna1-1", DutKind::AegisLuna1, vec!["jtag.mock"])],
            transport: TransportSection {
                jtag: vec![jtag_mock("jtag.mock")],
                gpio: vec![heimdall_config::GpioTransportCfg {
                    id: "gpio.host".into(),
                    driver: heimdall_config::GpioDriver::LinuxCdev,
                    device: Some(std::path::PathBuf::from("/dev/gpiochip0")),
                }],
                ..Default::default()
            },
            golden: GoldenCfg {
                aegis: Some(GoldenBackendCfg::Mock),
                river: None,
            },
            tools: ToolsCfg::default(),
            pad_maps: vec![heimdall_config::PadMapEntry {
                dut: "luna1-1".into(),
                direction: heimdall_config::PadDirection::In,
                fpga_pad: 0,
                gpio_line: 17,
                gpio_transport: "gpio.host".into(),
            }],
        };
        let reg = build_registry(&cfg).expect("registry");
        let r = reg.lookup(&DutId::new("luna1-1")).expect("dut");
        assert!(matches!(
            r.pad_map.gpio_spec,
            Some(GpioSpec::LinuxCdev { .. })
        ));
        if let Some(GpioSpec::LinuxCdev { device }) = &r.pad_map.gpio_spec {
            assert_eq!(device.to_str().unwrap(), "/dev/gpiochip0");
        }
    }

    #[test]
    fn pad_map_unknown_gpio_transport_errors() {
        let cfg = ConfigFile {
            host: host(),
            duts: vec![dut("luna1-1", DutKind::AegisLuna1, vec!["jtag.mock"])],
            transport: TransportSection {
                jtag: vec![jtag_mock("jtag.mock")],
                gpio: vec![],
                ..Default::default()
            },
            golden: GoldenCfg {
                aegis: Some(GoldenBackendCfg::Mock),
                river: None,
            },
            tools: ToolsCfg::default(),
            pad_maps: vec![heimdall_config::PadMapEntry {
                dut: "luna1-1".into(),
                direction: heimdall_config::PadDirection::In,
                fpga_pad: 0,
                gpio_line: 5,
                gpio_transport: "gpio.nowhere".into(),
            }],
        };
        let err = build_registry(&cfg).expect_err("should reject");
        assert!(
            format!("{err}").contains("unknown gpio transport"),
            "got: {err}"
        );
    }

    #[test]
    fn resolves_openocd_spawn() {
        let cfg = ConfigFile {
            host: host(),
            duts: vec![dut("luna1-1", DutKind::AegisLuna1, vec!["jtag.spawned"])],
            transport: TransportSection {
                jtag: vec![jtag_openocd_spawn("jtag.spawned")],
                ..Default::default()
            },
            golden: GoldenCfg {
                aegis: Some(GoldenBackendCfg::Mock),
                river: None,
            },
            tools: ToolsCfg::default(),
            pad_maps: vec![],
        };
        let reg = build_registry(&cfg).expect("registry");
        let r = reg.lookup(&DutId::new("luna1-1")).unwrap();
        match &r.jtag {
            TransportSpec::OpenocdSpawned {
                binary,
                config_file,
                tcl_port,
                extra_args,
            } => {
                assert_eq!(binary.to_str().unwrap(), "/usr/bin/openocd");
                assert_eq!(config_file.to_str().unwrap(), "/tmp/heimdall-test.cfg");
                assert_eq!(*tcl_port, 6666);
                assert_eq!(extra_args, &vec!["-d3".to_string()]);
            }
            other => panic!("unexpected spec {other:?}"),
        }
    }

    #[test]
    fn rejects_openocd_spawn_without_config_file() {
        let cfg = ConfigFile {
            host: host(),
            duts: vec![dut("luna1-1", DutKind::AegisLuna1, vec!["jtag.spawned"])],
            transport: TransportSection {
                jtag: vec![JtagTransportCfg {
                    id: "jtag.spawned".into(),
                    driver: JtagDriver::OpenocdSpawn,
                    serial: None,
                    openocd_endpoint: None,
                    openocd_config: None,
                    openocd_binary: None,
                    openocd_extra_args: vec![],
                    freq_hz: 1_000_000,
                    ftdi_vid: None,
                    ftdi_pid: None,
                    ftdi_interface: None,
                }],
                ..Default::default()
            },
            golden: GoldenCfg {
                aegis: Some(GoldenBackendCfg::Mock),
                river: None,
            },
            tools: ToolsCfg::default(),
            pad_maps: vec![],
        };
        let err = build_registry(&cfg).expect_err("should reject");
        let msg = format!("{err}");
        assert!(msg.contains("missing openocd_config"), "got: {msg}");
    }

    #[test]
    fn resolves_aegis_bringup_with_relative_paths() {
        let tmp = tempfile::TempDir::new().unwrap();
        let descriptor_path = tmp.path().join("desc.json");
        let bitstream_path = tmp.path().join("bits.bin");
        std::fs::write(&descriptor_path, b"{\"device\":\"test_fpga\"}").unwrap();
        std::fs::write(&bitstream_path, [0u8, 1, 2, 3]).unwrap();

        let cfg = ConfigFile {
            host: host(),
            duts: vec![heimdall_config::DutCfg {
                id: "luna1-1".into(),
                kind: DutKind::AegisLuna1,
                chip_serial: None,
                transports: vec!["jtag.mock".into()],
                expect_idcode: None,
                bringup: Some(heimdall_config::BringupSpec::AegisVector {
                    descriptor_path: std::path::PathBuf::from("desc.json"),
                    bitstream_path: std::path::PathBuf::from("bits.bin"),
                    inputs: std::collections::BTreeMap::new(),
                    expected_outputs: std::collections::BTreeMap::new(),
                    settle_cycles: 5,
                }),
                netlist: None,
                spice_watches: vec![],
            }],
            transport: TransportSection {
                jtag: vec![jtag_mock("jtag.mock")],
                ..Default::default()
            },
            golden: GoldenCfg {
                aegis: Some(GoldenBackendCfg::Mock),
                river: None,
            },
            tools: ToolsCfg::default(),
            pad_maps: vec![],
        };

        let reg = build_registry_with_root(&cfg, Some(tmp.path())).expect("registry");
        let r = reg.lookup(&DutId::new("luna1-1")).unwrap();
        match r.bringup.as_ref().unwrap() {
            BringupPayload::AegisVector {
                descriptor_json,
                bitstream,
                settle_cycles,
                ..
            } => {
                assert_eq!(descriptor_json, "{\"device\":\"test_fpga\"}");
                assert_eq!(bitstream, &[0u8, 1, 2, 3]);
                assert_eq!(*settle_cycles, 5);
            }
            other => panic!("unexpected payload {other:?}"),
        }
    }

    #[test]
    fn missing_bringup_file_rejected() {
        let cfg = ConfigFile {
            host: host(),
            duts: vec![heimdall_config::DutCfg {
                id: "luna1-1".into(),
                kind: DutKind::AegisLuna1,
                chip_serial: None,
                transports: vec!["jtag.mock".into()],
                expect_idcode: None,
                bringup: Some(heimdall_config::BringupSpec::RiverElf {
                    elf_path: std::path::PathBuf::from("/nonexistent/elf.bin"),
                    cycles: 1000,
                }),
                netlist: None,
                spice_watches: vec![],
            }],
            transport: TransportSection {
                jtag: vec![jtag_mock("jtag.mock")],
                ..Default::default()
            },
            golden: GoldenCfg {
                aegis: Some(GoldenBackendCfg::Mock),
                river: None,
            },
            tools: ToolsCfg::default(),
            pad_maps: vec![],
        };
        let err = build_registry(&cfg).expect_err("should reject");
        assert!(format!("{err}").contains("could not read"), "got: {err}");
    }
}
