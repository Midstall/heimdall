use async_trait::async_trait;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::caps::JtagOps;
use crate::error::TransportError;
use crate::traits::{ResetTarget, Result, Transport, TransportKind};

/// Talks to an externally-managed OpenOCD process over its Tcl RPC port.
/// OpenOCD must be started separately by the operator (or by the daemon's
/// process supervisor in a later plan).
///
/// The Tcl protocol terminates commands and responses with byte 0x1a.
const OPENOCD_DELIM: u8 = 0x1a;

pub struct OpenOcdJtagTransport {
    endpoint: SocketAddr,
    sock: Option<TcpStream>,
    tap_name: String,
}

/// Default tap name used by `JtagOps::shift_dr` when the caller doesn't
/// override via [`OpenOcdJtagTransport::with_tap_name`]. Matches the River
/// OpenOCD configs in the repo.
pub const DEFAULT_TAP_NAME: &str = "riscv.cpu";

impl OpenOcdJtagTransport {
    pub fn new(endpoint: SocketAddr) -> Self {
        Self {
            endpoint,
            sock: None,
            tap_name: DEFAULT_TAP_NAME.to_string(),
        }
    }

    /// Set the tap name `shift_dr` passes to OpenOCD's `irscan` / `drscan`.
    /// Must match a tap declared in the OpenOCD config (e.g. `aegis.cpu`).
    pub fn with_tap_name(mut self, name: impl Into<String>) -> Self {
        self.tap_name = name.into();
        self
    }

    pub fn tap_name(&self) -> &str {
        &self.tap_name
    }

    pub async fn rpc(&mut self, cmd: &str) -> Result<String> {
        let sock = self.sock.as_mut().ok_or(TransportError::NotOpen)?;
        sock.write_all(cmd.as_bytes()).await?;
        sock.write_all(&[OPENOCD_DELIM]).await?;
        sock.flush().await?;

        let mut out = Vec::new();
        let mut buf = [0u8; 1024];
        let read = async {
            loop {
                let n = sock.read(&mut buf).await?;
                if n == 0 {
                    return Err::<String, TransportError>(TransportError::OpenOcd(
                        "connection closed".into(),
                    ));
                }
                for &b in &buf[..n] {
                    if b == OPENOCD_DELIM {
                        return Ok(String::from_utf8_lossy(&out).into_owned());
                    }
                    out.push(b);
                }
            }
        };
        match timeout(Duration::from_secs(5), read).await {
            Ok(r) => r,
            Err(_) => Err(TransportError::Timeout { millis: 5000 }),
        }
    }
}

#[async_trait]
impl Transport for OpenOcdJtagTransport {
    fn kind(&self) -> TransportKind {
        TransportKind::Jtag
    }

    async fn open(&mut self) -> Result<()> {
        let sock = TcpStream::connect(self.endpoint).await?;
        // Disable Nagle's algorithm so small writes (like the 1-byte
        // 0x1a delimiter) are sent immediately rather than being buffered
        // while waiting for a delayed ACK. This keeps RPC latency low.
        sock.set_nodelay(true)?;
        self.sock = Some(sock);
        tracing::debug!(endpoint = %self.endpoint, "openocd connected");
        Ok(())
    }

    async fn close(&mut self) -> Result<()> {
        self.sock = None;
        Ok(())
    }

    async fn reset(&mut self, target: ResetTarget) -> Result<()> {
        let cmd = match target {
            ResetTarget::System => "reset run",
            ResetTarget::Cpu => "reset halt",
            ResetTarget::DebugModule => "reset init",
        };
        self.rpc(cmd).await?;
        Ok(())
    }
}

#[async_trait]
impl JtagOps for OpenOcdJtagTransport {
    async fn scan_idcode(&mut self) -> Result<Vec<u32>> {
        // OpenOCD "scan_chain" outputs lines like:
        //   1   river.cpu   Y   0xdeadbeef   ...
        let raw = self.rpc("scan_chain").await?;
        let mut out = Vec::new();
        for line in raw.lines() {
            for tok in line.split_whitespace() {
                if let Some(stripped) = tok.strip_prefix("0x") {
                    if let Ok(v) = u32::from_str_radix(stripped, 16) {
                        if v != 0 && v != 0xffff_ffff {
                            out.push(v);
                        }
                    }
                }
            }
        }
        Ok(out)
    }

    async fn shift_dr(&mut self, ir: u32, bits: usize, data: &[u8]) -> Result<Vec<u8>> {
        let hex_data = hex::encode(data);
        let tap = &self.tap_name;
        let cmd = format!("irscan {tap} 0x{ir:x}; drscan {tap} {bits} 0x{hex_data}");
        let raw = self.rpc(&cmd).await?;
        let trimmed = raw.trim();
        let stripped = trimmed.strip_prefix("0x").unwrap_or(trimmed);
        hex::decode(stripped).map_err(|e| TransportError::OpenOcd(format!("hex decode: {e}")))
    }
}

pub mod parse;
pub mod spawn;

/// Marker + dispatch trait for transports that speak OpenOCD's Tcl RPC.
/// Implemented for both the direct OpenOcdJtagTransport and the
/// daemon-spawned SpawnedOpenocdJtagTransport.
#[async_trait::async_trait]
pub trait OpenocdRpc: Send + Sync {
    async fn rpc(&mut self, cmd: &str) -> crate::traits::Result<String>;
}

#[async_trait::async_trait]
impl OpenocdRpc for OpenOcdJtagTransport {
    async fn rpc(&mut self, cmd: &str) -> crate::traits::Result<String> {
        OpenOcdJtagTransport::rpc(self, cmd).await
    }
}
