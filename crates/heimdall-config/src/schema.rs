use heimdall_core::DutKind;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ConfigFile {
    pub host: HostCfg,
    #[serde(default, rename = "dut")]
    pub duts: Vec<DutCfg>,
    #[serde(default)]
    pub transport: TransportSection,
    #[serde(default)]
    pub golden: GoldenCfg,
    #[serde(default)]
    pub tools: ToolsCfg,
    #[serde(default, rename = "pad_map")]
    pub pad_maps: Vec<PadMapEntry>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HostCfg {
    pub name: String,
    #[serde(default = "default_bind")]
    pub bind: SocketAddr,
}

fn default_bind() -> SocketAddr {
    "127.0.0.1:7777".parse().unwrap()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DutCfg {
    pub id: String,
    pub kind: DutKind,
    #[serde(default)]
    pub chip_serial: Option<String>,
    #[serde(default)]
    pub transports: Vec<String>,
    #[serde(default)]
    pub expect_idcode: Option<String>,
    #[serde(default)]
    pub bringup: Option<BringupSpec>,
    /// Path to a SPICE netlist describing this DUT (analog/mixed-signal
    /// flows). Optional. DUTs without a netlist still work; the
    /// `/duts/:id/netlist.svg` endpoint just returns 404 for them.
    #[serde(default)]
    pub netlist: Option<PathBuf>,
    /// Watches identifying which nets in the netlist are inputs/outputs.
    /// Drives the renderer's input/output highlighting.
    #[serde(default, rename = "spice_watch")]
    pub spice_watches: Vec<SpiceWatchCfg>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct SpiceWatchCfg {
    /// Heimdall-side pad/signal name (e.g., "io_2").
    pub name: String,
    /// Net name in the netlist (e.g., "n_pad_out_2").
    pub spice_node: String,
    pub direction: PadDirection,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct TransportSection {
    #[serde(default)]
    pub jtag: Vec<JtagTransportCfg>,
    #[serde(default)]
    pub uart: Vec<SerialTransportCfg>,
    #[serde(default)]
    pub usb: Vec<UsbTransportCfg>,
    #[serde(default)]
    pub psu: Vec<PsuTransportCfg>,
    #[serde(default)]
    pub gpio: Vec<GpioTransportCfg>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct JtagTransportCfg {
    pub id: String,
    pub driver: JtagDriver,
    #[serde(default)]
    pub serial: Option<String>,
    #[serde(default)]
    pub openocd_endpoint: Option<SocketAddr>,
    #[serde(default)]
    pub openocd_config: Option<PathBuf>,
    #[serde(default)]
    pub openocd_binary: Option<PathBuf>,
    #[serde(default)]
    pub openocd_extra_args: Vec<String>,
    #[serde(default = "default_jtag_freq")]
    pub freq_hz: u32,
    /// FTDI USB vendor ID. Defaults to 0x0403 (FTDI) when the driver is `ftdi`.
    #[serde(default)]
    pub ftdi_vid: Option<u16>,
    /// FTDI USB product ID. Defaults to 0x6010 (FT2232H) when the driver is `ftdi`.
    #[serde(default)]
    pub ftdi_pid: Option<u16>,
    /// MPSSE channel on multi-port FTDI parts: 0/1 for FT2232H, 0..3 for FT4232H.
    #[serde(default)]
    pub ftdi_interface: Option<u8>,
}

fn default_jtag_freq() -> u32 {
    1_000_000
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum JtagDriver {
    Ftdi,
    Openocd,
    OpenocdSpawn,
    BitbangJtag,
    Mock,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SerialTransportCfg {
    pub id: String,
    pub path: PathBuf,
    #[serde(default = "default_baud")]
    pub baud: u32,
    #[serde(default)]
    pub driver: UartDriver,
}

fn default_baud() -> u32 {
    115_200
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum UartDriver {
    #[default]
    System,
    Mock,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UsbTransportCfg {
    pub id: String,
    pub vid: u16,
    pub pid: u16,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PsuTransportCfg {
    pub id: String,
    pub endpoint: String,
    #[serde(default = "default_psu_channel")]
    pub channel: u8,
    #[serde(default)]
    pub ocp_amps: Option<f32>,
}

fn default_psu_channel() -> u8 {
    1
}

/// Reference to a transport by id, qualified by family.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportRef(pub String);

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct GoldenCfg {
    #[serde(default)]
    pub river: Option<GoldenBackendCfg>,
    #[serde(default)]
    pub aegis: Option<GoldenBackendCfg>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "backend", rename_all = "kebab-case")]
pub enum GoldenBackendCfg {
    SpikeOneShot {
        binary: PathBuf,
        #[serde(default)]
        extra_args: Vec<String>,
    },
    DartRpc {
        entry: PathBuf,
        #[serde(default)]
        env: BTreeMap<String, String>,
    },
    Mock,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ToolsCfg {
    #[serde(default)]
    pub clang_riscv64: Option<PathBuf>,
    #[serde(default)]
    pub yosys: Option<PathBuf>,
    #[serde(default)]
    pub dart: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum PadDirection {
    In,
    Out,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PadMapEntry {
    pub dut: String,
    pub direction: PadDirection,
    pub fpga_pad: u32,
    pub gpio_line: u32,
    pub gpio_transport: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GpioTransportCfg {
    pub id: String,
    pub driver: GpioDriver,
    #[serde(default)]
    pub device: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum GpioDriver {
    LinuxCdev,
    Mock,
}

/// Per-DUT bringup vector descriptor. Paths can be absolute or relative;
/// relative paths resolve against the directory containing heimdall.toml.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case", tag = "type")]
pub enum BringupSpec {
    /// Aegis: load a bitstream, drive input pads, observe outputs.
    AegisVector {
        descriptor_path: PathBuf,
        bitstream_path: PathBuf,
        #[serde(default)]
        inputs: BTreeMap<String, bool>,
        #[serde(default)]
        expected_outputs: BTreeMap<String, bool>,
        #[serde(default = "default_settle_cycles")]
        settle_cycles: u64,
    },
    /// River: boot a precompiled ELF, run for N cycles, observe state.
    RiverElf {
        elf_path: PathBuf,
        #[serde(default = "default_cycles")]
        cycles: u64,
    },
}

fn default_settle_cycles() -> u64 {
    1
}

fn default_cycles() -> u64 {
    10_000
}
