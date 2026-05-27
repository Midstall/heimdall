use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("io error reading {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("toml parse error in {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("dut `{dut}` references undefined transport id `{transport}`")]
    UnknownTransportRef { dut: String, transport: String },
    #[error("duplicate transport id `{0}`")]
    DuplicateTransportId(String),
    #[error("duplicate dut id `{0}`")]
    DuplicateDutId(String),
    #[error("no golden backend declared for dut family of `{dut}` ({kind:?})")]
    MissingGoldenBackend {
        dut: String,
        kind: heimdall_core::DutKind,
    },
    #[error("pad_map entry references unknown dut id `{0}`")]
    PadMapUnknownDut(String),
    #[error("pad_map entry references unknown gpio transport id `{0}`")]
    PadMapUnknownGpioTransport(String),
    #[error("duplicate pad_map entry: dut=`{dut}` direction={direction:?} fpga_pad={fpga_pad}")]
    DuplicatePadMap {
        dut: String,
        direction: crate::schema::PadDirection,
        fpga_pad: u32,
    },
    #[error("duplicate gpio transport id `{0}`")]
    DuplicateGpioTransportId(String),
    #[error("duplicate spice_watch `{name}` on dut `{dut}`")]
    DuplicateSpiceWatch { dut: String, name: String },
}
