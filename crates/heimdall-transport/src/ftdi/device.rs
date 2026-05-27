//! Real FTDI MPSSE backend over USB via `nusb`. Pure Rust.
//!
//! Supports the common MPSSE-capable parts: FT2232H (0x0403:0x6010),
//! FT232H (0x0403:0x6014), FT4232H (0x0403:0x6011). The interface number
//! selects the MPSSE channel: 0 for the single-port FT232H, 0 or 1 for
//! FT2232H, 0..3 for FT4232H.
//!
//! Hardware-in-the-loop tests live in `tests/ftdi_hardware.rs` and are
//! `#[ignore]`d by default. Run them with `cargo test -p heimdall-transport
//! --features ftdi -- --ignored ftdi_hardware`.

use std::time::Duration;

use nusb::transfer::{ControlOut, ControlType, Recipient, RequestBuffer};

use crate::error::TransportError;
use crate::ftdi::mpsse::MpsseBackend;
use crate::traits::Result;

/// FTDI vendor-specific USB control request numbers (subset).
mod ctrl {
    pub const RESET: u8 = 0x00;
    pub const SET_LATENCY_TIMER: u8 = 0x09;
    pub const SET_BITMODE: u8 = 0x0B;
}

/// SIO Reset args.
const SIO_RESET_PURGE_RX: u16 = 1;
const SIO_RESET_PURGE_TX: u16 = 2;
const SIO_RESET_SIO: u16 = 0;

/// Bitmode values.
const BITMODE_RESET: u8 = 0x00;
const BITMODE_MPSSE: u8 = 0x02;

/// Default IO timeout for bulk USB transfers.
pub const DEFAULT_IO_TIMEOUT: Duration = Duration::from_millis(1500);

/// USB-level configuration for a single FTDI MPSSE channel.
#[derive(Debug, Clone)]
pub struct FtdiUsbConfig {
    pub vid: u16,
    pub pid: u16,
    /// Optional serial number filter; matches any device if `None`.
    pub serial: Option<String>,
    /// MPSSE channel: 0 for FT232H, 0/1 for FT2232H, 0..3 for FT4232H.
    pub interface: u8,
    /// Bulk transfer timeout.
    pub timeout: Duration,
}

impl Default for FtdiUsbConfig {
    fn default() -> Self {
        Self {
            vid: 0x0403,
            pid: 0x6010, // FT2232H
            serial: None,
            interface: 0,
            timeout: DEFAULT_IO_TIMEOUT,
        }
    }
}

/// Lightweight presence info for one connected FTDI-compatible device.
/// Returned by [`enumerate_ftdi_devices`]; no USB interface is claimed.
#[derive(Debug, Clone)]
pub struct FtdiDeviceInfo {
    pub vid: u16,
    pub pid: u16,
    pub serial: Option<String>,
}

/// Enumerate USB devices that match `vid:pid`, or any device with FTDI's
/// vendor id (0x0403) when `vid` is `None`. Filters by `serial` if set.
///
/// This is a non-claiming probe used to answer "is the DUT plugged in?".
/// Returns an empty Vec on host-USB enumeration errors (treated as "we
/// don't know, fall through to Unknown" by callers).
pub fn enumerate_ftdi_devices(
    vid: Option<u16>,
    pid: Option<u16>,
    serial: Option<&str>,
) -> Vec<FtdiDeviceInfo> {
    let Ok(devices) = nusb::list_devices() else {
        return Vec::new();
    };
    devices
        .filter_map(|d| {
            let v = d.vendor_id();
            let p = d.product_id();
            let want_v = vid.unwrap_or(0x0403);
            if v != want_v {
                return None;
            }
            if let Some(want_p) = pid {
                if p != want_p {
                    return None;
                }
            }
            let dev_serial = d.serial_number().map(str::to_owned);
            if let Some(want_s) = serial {
                if dev_serial.as_deref() != Some(want_s) {
                    return None;
                }
            }
            Some(FtdiDeviceInfo {
                vid: v,
                pid: p,
                serial: dev_serial,
            })
        })
        .collect()
}

/// nusb-backed MPSSE backend. Owns the USB interface and the in/out endpoints.
pub struct FtdiNusbBackend {
    interface: nusb::Interface,
    in_ep: u8,
    out_ep: u8,
    timeout: Duration,
    /// 2-byte modem-status header is prepended to every IN packet by the
    /// FTDI chip; we strip it from each chunk read.
    leftover_in: Vec<u8>,
}

impl FtdiNusbBackend {
    /// Open the FTDI device matching `cfg`, claim its MPSSE interface, and
    /// switch it into MPSSE mode. Returns a ready-to-use backend.
    pub fn open(cfg: &FtdiUsbConfig) -> Result<Self> {
        let info = nusb::list_devices()
            .map_err(io_err)?
            .find(|d| {
                d.vendor_id() == cfg.vid
                    && d.product_id() == cfg.pid
                    && cfg
                        .serial
                        .as_ref()
                        .map(|s| d.serial_number() == Some(s.as_str()))
                        .unwrap_or(true)
            })
            .ok_or_else(|| {
                TransportError::DeviceNotFound(format!(
                    "no FTDI device matching VID=0x{:04x} PID=0x{:04x} serial={:?}",
                    cfg.vid, cfg.pid, cfg.serial
                ))
            })?;

        let device = info.open().map_err(io_err)?;
        let interface = device.claim_interface(cfg.interface).map_err(io_err)?;

        // FTDI bulk endpoints by interface (per AN_232B-01 §3.2). Interface
        // 0 uses 0x02/0x81; interface 1 uses 0x04/0x83; etc. Hard-coded
        // because nusb's descriptor APIs are awkward to traverse and this
        // mapping is fixed across all multi-port FTDI parts.
        let (out_ep, in_ep) = match cfg.interface {
            0 => (0x02, 0x81),
            1 => (0x04, 0x83),
            2 => (0x06, 0x85),
            3 => (0x08, 0x87),
            n => {
                return Err(TransportError::Protocol(format!(
                    "unsupported FTDI interface index {n} (must be 0..=3)"
                )));
            }
        };

        let mut backend = Self {
            interface,
            in_ep,
            out_ep,
            timeout: cfg.timeout,
            leftover_in: Vec::new(),
        };

        backend.reset_chip()?;
        backend.set_latency_timer(2)?;
        backend.set_bitmode(BITMODE_RESET, 0x00)?;
        backend.set_bitmode(BITMODE_MPSSE, 0x0B)?;
        // Purge any residual data from prior sessions.
        backend.purge_rx()?;
        backend.purge_tx()?;

        Ok(backend)
    }

    fn reset_chip(&mut self) -> Result<()> {
        self.control_out(ctrl::RESET, SIO_RESET_SIO)
    }

    fn purge_rx(&mut self) -> Result<()> {
        self.leftover_in.clear();
        self.control_out(ctrl::RESET, SIO_RESET_PURGE_RX)
    }

    fn purge_tx(&mut self) -> Result<()> {
        self.control_out(ctrl::RESET, SIO_RESET_PURGE_TX)
    }

    fn set_latency_timer(&mut self, ms: u8) -> Result<()> {
        self.control_out(ctrl::SET_LATENCY_TIMER, ms as u16)
    }

    fn set_bitmode(&mut self, mode: u8, mask: u8) -> Result<()> {
        let value = ((mode as u16) << 8) | (mask as u16);
        self.control_out(ctrl::SET_BITMODE, value)
    }

    fn control_out(&mut self, request: u8, value: u16) -> Result<()> {
        let interface_idx = self.interface_index_word();
        let ctrl = ControlOut {
            control_type: ControlType::Vendor,
            recipient: Recipient::Device,
            request,
            value,
            index: interface_idx,
            data: &[],
        };
        futures::executor::block_on(async {
            self.interface
                .control_out(ctrl)
                .await
                .into_result()
                .map(|_| ())
        })
        .map_err(io_err)
    }

    /// FTDI control-transfer wIndex encoding: low byte = port (interface+1).
    fn interface_index_word(&self) -> u16 {
        // nusb's claim_interface uses bInterfaceNumber. Convert to FTDI's
        // 1-based port number.
        // For interface 0 -> port 1, etc.
        // We don't have direct access here, so re-derive from out_ep.
        match self.out_ep {
            0x02 => 1,
            0x04 => 2,
            0x06 => 3,
            0x08 => 4,
            _ => 1,
        }
    }
}

impl MpsseBackend for FtdiNusbBackend {
    fn write_all(&mut self, data: &[u8]) -> Result<()> {
        let buf = data.to_vec();
        let out_ep = self.out_ep;
        futures::executor::block_on(async {
            self.interface
                .bulk_out(out_ep, buf)
                .await
                .into_result()
                .map(|_| ())
        })
        .map_err(io_err)
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> Result<()> {
        let deadline = std::time::Instant::now() + self.timeout;
        let mut filled = 0usize;
        while filled < buf.len() {
            // Drain leftover first.
            if !self.leftover_in.is_empty() {
                let take = self.leftover_in.len().min(buf.len() - filled);
                buf[filled..filled + take].copy_from_slice(&self.leftover_in[..take]);
                self.leftover_in.drain(..take);
                filled += take;
                continue;
            }
            if std::time::Instant::now() >= deadline {
                return Err(TransportError::Timeout {
                    millis: self.timeout.as_millis() as u64,
                });
            }
            // Read one bulk packet. FTDI prepends a 2-byte modem-status
            // header to every IN packet; we drop those bytes.
            let chunk = futures::executor::block_on(async {
                let req = RequestBuffer::new(512);
                self.interface.bulk_in(self.in_ep, req).await.into_result()
            })
            .map_err(io_err)?;
            if chunk.len() < 2 {
                continue;
            }
            self.leftover_in.extend_from_slice(&chunk[2..]);
        }
        Ok(())
    }
}

fn io_err<E: std::fmt::Display>(e: E) -> TransportError {
    TransportError::Io(std::io::Error::other(e.to_string()))
}
