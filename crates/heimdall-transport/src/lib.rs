//! Transport abstractions for talking to physical DUTs.

pub mod caps;
pub mod error;
pub mod traits;

#[cfg(feature = "bitbang-jtag")]
pub mod bitbang_jtag;
#[cfg(feature = "ftdi")]
pub mod ftdi;
#[cfg(all(feature = "linux-cdev", target_os = "linux"))]
pub mod gpio_cdev;
#[cfg(feature = "mock")]
pub mod mock;
#[cfg(feature = "openocd")]
pub mod openocd;
#[cfg(feature = "serial")]
pub mod serial;

pub use caps::{GpioOps, GpioTransport, JtagOps, LogicAnalyzerOps, PsuOps, SerialOps, UsbOps};
pub use error::TransportError;
pub use traits::{ResetTarget, Transport, TransportKind};
