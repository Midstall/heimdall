//! heimdall.toml parsing, schema, and validation.

pub mod error;
pub mod load;
pub mod schema;
pub mod validate;

pub use error::ConfigError;
pub use load::load_from_path;
pub use schema::{
    BringupSpec, ConfigFile, DutCfg, GoldenBackendCfg, GoldenCfg, GpioDriver, GpioTransportCfg,
    HostCfg, JtagDriver, JtagTransportCfg, PadDirection, PadMapEntry, PsuTransportCfg,
    SerialTransportCfg, SpiceWatchCfg, ToolsCfg, TransportRef, TransportSection, UartDriver,
    UsbTransportCfg,
};
