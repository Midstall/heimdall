use async_trait::async_trait;
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::timeout;
use tokio_serial::{SerialPortBuilderExt, SerialStream};

use crate::caps::SerialOps;
use crate::error::TransportError;
use crate::traits::{ResetTarget, Result, Transport, TransportKind};

pub struct SerialTransport {
    path: PathBuf,
    baud: u32,
    port: Option<SerialStream>,
}

impl SerialTransport {
    pub fn new(path: impl Into<PathBuf>, baud: u32) -> Self {
        Self {
            path: path.into(),
            baud,
            port: None,
        }
    }

    fn port_mut(&mut self) -> Result<&mut SerialStream> {
        self.port.as_mut().ok_or(TransportError::NotOpen)
    }
}

#[async_trait]
impl Transport for SerialTransport {
    fn kind(&self) -> TransportKind {
        TransportKind::Serial
    }

    async fn open(&mut self) -> Result<()> {
        let port = tokio_serial::new(self.path.to_string_lossy(), self.baud)
            .open_native_async()
            .map_err(|e| TransportError::Io(std::io::Error::other(e)))?;
        self.port = Some(port);
        tracing::debug!(path = %self.path.display(), baud = self.baud, "serial open");
        Ok(())
    }

    async fn close(&mut self) -> Result<()> {
        self.port = None;
        Ok(())
    }

    async fn reset(&mut self, _target: ResetTarget) -> Result<()> {
        // SerialTransport on its own can only toggle DTR/RTS; we leave
        // reset-via-control-lines as a future enhancement. Return ok so that
        // drivers that compose serial + jtag can no-op the serial side.
        Ok(())
    }
}

#[async_trait]
impl SerialOps for SerialTransport {
    async fn write_all(&mut self, bytes: &[u8]) -> Result<()> {
        let port = self.port_mut()?;
        port.write_all(bytes).await?;
        port.flush().await?;
        Ok(())
    }

    async fn read_until(&mut self, delim: u8, dur: Duration) -> Result<Vec<u8>> {
        let port = self.port_mut()?;
        let fut = async {
            let mut out = Vec::new();
            let mut buf = [0u8; 256];
            loop {
                let n = port.read(&mut buf).await?;
                if n == 0 {
                    return Ok::<_, TransportError>(out);
                }
                for &b in &buf[..n] {
                    out.push(b);
                    if b == delim {
                        return Ok(out);
                    }
                }
            }
        };
        match timeout(dur, fut).await {
            Ok(r) => r,
            Err(_) => Err(TransportError::Timeout {
                millis: dur.as_millis() as u64,
            }),
        }
    }
}
