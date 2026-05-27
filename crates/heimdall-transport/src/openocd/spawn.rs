//! Spawned OpenOCD process + inner OpenOcdJtagTransport. Useful when the
//! daemon should manage the OpenOCD lifecycle itself (Pi+linuxgpio+RV-debug
//! deployments).

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use tokio::net::TcpStream;
use tokio::process::{Child, Command};
use tokio::time::{Instant, sleep, timeout};
use tracing::{debug, info, warn};

use crate::caps::JtagOps;
use crate::error::TransportError;
use crate::openocd::OpenOcdJtagTransport;
use crate::traits::{ResetTarget, Result, Transport, TransportKind};

/// Spawns OpenOCD with a given config file + Tcl port, polls the port until
/// reachable, then delegates all JTAG ops to an inner `OpenOcdJtagTransport`.
/// On `close`, terminates the subprocess.
pub struct SpawnedOpenocdJtagTransport {
    binary: PathBuf,
    config_file: PathBuf,
    tcl_port: u16,
    extra_args: Vec<String>,
    tap_name: String,
    /// Maximum wait for the Tcl port to come up.
    pub startup_timeout: Duration,
    inner: Option<Inner>,
}

struct Inner {
    child: Child,
    transport: OpenOcdJtagTransport,
}

impl SpawnedOpenocdJtagTransport {
    pub fn new(binary: impl Into<PathBuf>, config_file: impl Into<PathBuf>, tcl_port: u16) -> Self {
        Self {
            binary: binary.into(),
            config_file: config_file.into(),
            tcl_port,
            extra_args: Vec::new(),
            tap_name: crate::openocd::DEFAULT_TAP_NAME.to_string(),
            startup_timeout: Duration::from_secs(10),
            inner: None,
        }
    }

    pub fn with_extra_args(mut self, args: impl IntoIterator<Item = String>) -> Self {
        self.extra_args.extend(args);
        self
    }

    /// Forward a tap name to the inner [`OpenOcdJtagTransport`]'s `shift_dr`.
    pub fn with_tap_name(mut self, name: impl Into<String>) -> Self {
        self.tap_name = name.into();
        self
    }

    pub fn tap_name(&self) -> &str {
        &self.tap_name
    }

    pub fn with_startup_timeout(mut self, d: Duration) -> Self {
        self.startup_timeout = d;
        self
    }

    fn inner_mut(&mut self) -> Result<&mut OpenOcdJtagTransport> {
        self.inner
            .as_mut()
            .map(|i| &mut i.transport)
            .ok_or(TransportError::NotOpen)
    }

    /// Pass-through to the inner OpenOcdJtagTransport's `rpc`. The transport
    /// must be open, otherwise returns `TransportError::NotOpen`.
    pub async fn rpc(&mut self, cmd: &str) -> Result<String> {
        self.inner_mut()?.rpc(cmd).await
    }

    /// Poll the Tcl port until a TCP connection succeeds or the deadline
    /// elapses. Useful as the readiness check after spawning OpenOCD.
    async fn wait_for_port(addr: std::net::SocketAddr, deadline: Instant) -> Result<()> {
        loop {
            if Instant::now() >= deadline {
                return Err(TransportError::Timeout {
                    millis: deadline.elapsed().as_millis() as u64,
                });
            }
            match TcpStream::connect(addr).await {
                Ok(s) => {
                    drop(s);
                    return Ok(());
                }
                Err(_) => {
                    sleep(Duration::from_millis(50)).await;
                }
            }
        }
    }
}

#[async_trait]
impl Transport for SpawnedOpenocdJtagTransport {
    fn kind(&self) -> TransportKind {
        TransportKind::Jtag
    }

    async fn open(&mut self) -> Result<()> {
        if self.inner.is_some() {
            return Ok(());
        }

        // Build the command: openocd -f <config> -c "tcl_port <port>"
        // -c "bindto 127.0.0.1" -c "init"
        let mut cmd = Command::new(&self.binary);
        cmd.arg("-f").arg(&self.config_file);
        cmd.arg("-c").arg(format!("tcl_port {}", self.tcl_port));
        cmd.arg("-c").arg("bindto 127.0.0.1");
        cmd.arg("-c").arg("init");
        for a in &self.extra_args {
            cmd.arg(a);
        }
        cmd.kill_on_drop(true);
        debug!(
            binary = %self.binary.display(),
            config = %self.config_file.display(),
            port = self.tcl_port,
            "spawning openocd"
        );
        let child = cmd.spawn().map_err(TransportError::Io)?;

        // Poll the port.
        let endpoint: std::net::SocketAddr = format!("127.0.0.1:{}", self.tcl_port)
            .parse()
            .expect("valid socket addr");
        let deadline = Instant::now() + self.startup_timeout;
        let wait = Self::wait_for_port(endpoint, deadline);
        if let Err(e) = timeout(self.startup_timeout, wait)
            .await
            .unwrap_or(Err(TransportError::Timeout { millis: 0 }))
        {
            // Tear down the orphan subprocess on timeout.
            let mut child = child;
            let _ = child.start_kill();
            let _ = child.wait().await;
            return Err(e);
        }

        let mut transport =
            OpenOcdJtagTransport::new(endpoint).with_tap_name(self.tap_name.clone());
        transport.open().await?;

        self.inner = Some(Inner { child, transport });
        info!(port = self.tcl_port, "spawned openocd ready");
        Ok(())
    }

    async fn close(&mut self) -> Result<()> {
        if let Some(mut inner) = self.inner.take() {
            // Close inner connection first so OpenOCD knows the client left.
            let _ = inner.transport.close().await;
            // Then kill the subprocess.
            if let Err(e) = inner.child.start_kill() {
                warn!(error = %e, "could not signal openocd child");
            }
            let _ = inner.child.wait().await;
        }
        Ok(())
    }

    async fn reset(&mut self, target: ResetTarget) -> Result<()> {
        self.inner_mut()?.reset(target).await
    }
}

#[async_trait]
impl JtagOps for SpawnedOpenocdJtagTransport {
    async fn scan_idcode(&mut self) -> Result<Vec<u32>> {
        self.inner_mut()?.scan_idcode().await
    }
    async fn shift_dr(&mut self, ir: u32, bits: usize, data: &[u8]) -> Result<Vec<u8>> {
        self.inner_mut()?.shift_dr(ir, bits, data).await
    }
}

#[async_trait]
impl crate::openocd::OpenocdRpc for SpawnedOpenocdJtagTransport {
    async fn rpc(&mut self, cmd: &str) -> Result<String> {
        self.rpc(cmd).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that when the spawned binary does NOT bind the tcl port (we
    /// use `sleep`), open() reports a timeout error.
    #[tokio::test]
    async fn open_times_out_when_binary_never_binds() {
        let mut t = SpawnedOpenocdJtagTransport::new(
            "sleep",
            "/dev/null",
            // Pick an obscure port that's unlikely to be bound.
            55321,
        )
        .with_extra_args(["30".into()])
        .with_startup_timeout(Duration::from_millis(300));
        let err = t.open().await.expect_err("expected timeout");
        assert!(matches!(err, TransportError::Timeout { .. }), "got {err:?}");

        // Tear down the orphan child if one is still attached.
        let _ = t.close().await;
    }

    /// Verify that if the binary can't be found at all, open() returns an Io
    /// error.
    #[tokio::test]
    async fn open_returns_io_when_binary_missing() {
        let mut t = SpawnedOpenocdJtagTransport::new(
            "/nonexistent/path/to/openocd-binary-does-not-exist",
            "/dev/null",
            55322,
        )
        .with_startup_timeout(Duration::from_millis(100));
        let err = t.open().await.expect_err("expected io error");
        assert!(matches!(err, TransportError::Io(_)), "got {err:?}");
    }
}
