//! Minimal RISC-V debug-module wrapper backed by an OpenocdRpc transport
//! (either OpenOcdJtagTransport or SpawnedOpenocdJtagTransport). Higher-level
//! orchestration; OpenOCD does the actual RV-debug bit-banging.

use std::time::Duration;

use heimdall_transport::TransportError;
use heimdall_transport::openocd::OpenocdRpc;
use heimdall_transport::openocd::parse::{parse_load_image_response, parse_reg_response};
use tempfile::NamedTempFile;

/// Map a RISC-V architectural GPR index (0-31) to the name OpenOCD's RISC-V
/// driver uses in its register cache. Matches `gpr_names[]` in OpenOCD's
/// `riscv-013.c`. Note `fp` for x8 rather than the equally valid `s0`.
pub fn gpr_abi_name(reg: u8) -> Option<&'static str> {
    Some(match reg {
        0 => "zero",
        1 => "ra",
        2 => "sp",
        3 => "gp",
        4 => "tp",
        5 => "t0",
        6 => "t1",
        7 => "t2",
        8 => "fp",
        9 => "s1",
        10 => "a0",
        11 => "a1",
        12 => "a2",
        13 => "a3",
        14 => "a4",
        15 => "a5",
        16 => "a6",
        17 => "a7",
        18 => "s2",
        19 => "s3",
        20 => "s4",
        21 => "s5",
        22 => "s6",
        23 => "s7",
        24 => "s8",
        25 => "s9",
        26 => "s10",
        27 => "s11",
        28 => "t3",
        29 => "t4",
        30 => "t5",
        31 => "t6",
        _ => return None,
    })
}

pub struct DebugModule<'a, T: OpenocdRpc> {
    pub jtag: &'a mut T,
}

impl<'a, T: OpenocdRpc> DebugModule<'a, T> {
    pub fn new(jtag: &'a mut T) -> Self {
        Self { jtag }
    }

    pub async fn halt(&mut self) -> Result<(), TransportError> {
        self.jtag.rpc("halt").await?;
        Ok(())
    }

    pub async fn resume(&mut self) -> Result<(), TransportError> {
        self.jtag.rpc("resume").await?;
        Ok(())
    }

    pub async fn wait_halt(&mut self, timeout: Duration) -> Result<(), TransportError> {
        let cmd = format!("wait_halt {}", timeout.as_millis());
        let raw = self.jtag.rpc(&cmd).await?;
        let lower = raw.to_ascii_lowercase();
        if lower.contains("timed out") || lower.contains("timeout") {
            return Err(TransportError::Timeout {
                millis: timeout.as_millis() as u64,
            });
        }
        Ok(())
    }

    pub async fn read_gpr(&mut self, reg: u8) -> Result<u64, TransportError> {
        // OpenOCD's RISC-V register cache is keyed by ABI name, not `xN`,
        // so `reg x1` returns "register x1 not found in current target".
        let name = gpr_abi_name(reg)
            .ok_or_else(|| TransportError::Protocol(format!("invalid RV GPR index {reg}")))?;
        let raw = self.jtag.rpc(&format!("reg {name}")).await?;
        Ok(parse_reg_response(&raw)?)
    }

    pub async fn read_csr(&mut self, name: &str) -> Result<u64, TransportError> {
        let raw = self.jtag.rpc(&format!("reg {name}")).await?;
        Ok(parse_reg_response(&raw)?)
    }

    pub async fn write_mem(&mut self, addr: u64, bytes: &[u8]) -> Result<usize, TransportError> {
        let mut file = NamedTempFile::new()?;
        use std::io::Write as _;
        file.write_all(bytes)?;
        let path = file.into_temp_path();
        let cmd = format!("load_image {} 0x{:x} bin", path.display(), addr);
        let raw = self.jtag.rpc(&cmd).await?;
        Ok(parse_load_image_response(&raw)?)
    }
}
