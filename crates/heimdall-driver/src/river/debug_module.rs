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

/// DMI address of `dmcontrol` per the RISC-V External Debug Spec.
const DMI_DMCONTROL: u32 = 0x10;
/// `dmcontrol.dmactive` bit. Setting this from 0 to 1 reinitialises the
/// debug module without touching the upstream JTAG adapter.
const DMCONTROL_DMACTIVE: u32 = 1 << 0;

/// DMI address of `data0`, the low half of the abstract-command data
/// register. For 64-bit access-register reads the value lands in
/// `data0:data1` (low:high).
const DMI_DATA0: u32 = 0x04;
/// DMI address of `data1`, the high half of the abstract-command data
/// register on RV64 access-register reads.
const DMI_DATA1: u32 = 0x05;
/// DMI address of `abstractcs`. Used to poll `busy` and read `cmderr`.
const DMI_ABSTRACTCS: u32 = 0x16;
/// DMI address of `command`. Writing here kicks off an abstract command.
const DMI_COMMAND: u32 = 0x17;

/// `abstractcs.busy` bit. Set while the DM is processing an abstract
/// command; the host must wait for it to clear before reading data
/// registers or issuing the next command.
const ABSTRACTCS_BUSY: u32 = 1 << 12;
/// `abstractcs.cmderr` mask (bits 10:8). Non-zero means the previous
/// command failed; values: 1=busy-on-issue, 2=not-supported,
/// 3=exception, 4=halt/resume, 5=bus, 6=reserved, 7=other.
const ABSTRACTCS_CMDERR_MASK: u32 = 0b111 << 8;
const ABSTRACTCS_CMDERR_SHIFT: u32 = 8;

/// Access-Register command body for an RV64 register READ (cmdtype=0).
/// Layout (RISC-V Debug Spec 0.13.2 Table 3.6):
///   bits 23:20: aarsize = 3 (8 bytes / 64-bit)
///   bit  19   : aarpostincrement = 0
///   bit  18   : postexec = 0
///   bit  17   : transfer = 1 (do the actual access)
///   bit  16   : write = 0 (read)
///   bits 15:0 : regno
fn access_register_read_rv64(regno: u16) -> u32 {
    (3u32 << 20) | (1u32 << 17) | regno as u32
}

/// Maximum number of `abstractcs` polls while waiting for `busy` to
/// drop. Tuned generously: a healthy DM clears `busy` in well under
/// a millisecond; if it has not cleared after this many polls the
/// target is either wedged or the debug-bus link is broken and we
/// want to surface the timeout rather than spin forever.
const ABSTRACTCS_POLL_LIMIT: u32 = 4096;

/// Convert a RISC-V architectural GPR index (0..31) to its abstract-
/// command regno per the Debug Spec: GPRs sit at 0x1000..0x101f, so
/// `regno = 0x1000 | reg`.
pub fn gpr_abstract_regno(reg: u8) -> u16 {
    0x1000 | (reg as u16 & 0x1f)
}

/// CSR regno for `dpc` (Debug PC). Read this when the core is halted
/// in Debug Mode to get the PC at the point of halt; the architectural
/// `pc` register has no abstract-command regno of its own.
pub const CSR_REGNO_DPC: u16 = 0x7b1;

impl<'a, T: OpenocdRpc> DebugModule<'a, T> {
    pub fn new(jtag: &'a mut T) -> Self {
        Self { jtag }
    }

    /// Reset the DM by cycling `dmcontrol.dmactive` over DMI. Unlike OpenOCD's
    /// `reset init`, this does NOT re-run examine or churn the adapter, so it
    /// survives slow remote_bitbang links (River HDL sim) that drop on heavy
    /// reset handshakes.
    pub async fn dm_reset(&mut self) -> Result<(), TransportError> {
        self.jtag
            .rpc(&format!("riscv dmi_write 0x{DMI_DMCONTROL:x} 0x0"))
            .await?;
        self.jtag
            .rpc(&format!(
                "riscv dmi_write 0x{DMI_DMCONTROL:x} 0x{DMCONTROL_DMACTIVE:x}"
            ))
            .await?;
        let raw = self
            .jtag
            .rpc(&format!("riscv dmi_read 0x{DMI_DMCONTROL:x}"))
            .await?;
        let trimmed = raw.trim();
        let value = parse_hex_word(trimmed).ok_or_else(|| {
            TransportError::Protocol(format!("dm_reset: unparseable dmcontrol `{trimmed}`"))
        })?;
        if value & DMCONTROL_DMACTIVE == 0 {
            return Err(TransportError::Protocol(format!(
                "dm_reset: dmactive never came up (dmcontrol=0x{value:08x})"
            )));
        }
        Ok(())
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

    /// Write a CPU register (GPR or CSR) by name via OpenOCD's `reg`
    /// command. Used by [`RiverCpuDriver::run`] to seed `dpc` with the ELF
    /// entry before resuming.
    pub async fn write_reg(&mut self, name: &str, value: u64) -> Result<(), TransportError> {
        // OpenOCD treats the second arg as hex when prefixed with 0x. Keep
        // the format consistent so the RPC echo doesn't surprise parsers.
        self.jtag.rpc(&format!("reg {name} 0x{value:x}")).await?;
        Ok(())
    }

    /// Read a register's 64-bit value via the DM's abstract-command
    /// interface, bypassing OpenOCD's register cache.
    ///
    /// Some OpenOCD builds (verified against nixpkgs openocd-0.12.0,
    /// 2026-06-02) don't refresh their register cache when the core
    /// self-halts via `ebreak`, so `reg <name>` returns stale values
    /// without ever issuing an access-register command on the wire.
    /// Going through DMI directly sidesteps the cache: we write the
    /// `command` register, wait for `abstractcs.busy` to clear, check
    /// `cmderr`, and read `data0`/`data1` from the DM. The bytes that
    /// come back are whatever the target's DM actually holds right
    /// now, not whatever OpenOCD remembers.
    ///
    /// `regno` is the Debug-Spec register number:
    /// - GPR x<i> -> [`gpr_abstract_regno`] (0x1000 | i)
    /// - CSR <addr> -> the CSR address itself (e.g. dpc -> 0x7b1)
    ///
    /// Caller must ensure the core is halted in Debug Mode before
    /// calling. Reading registers from a running core is implementation-
    /// defined and on most cores returns `cmderr=halt/resume`.
    pub async fn read_register_via_abstract(&mut self, regno: u16) -> Result<u64, TransportError> {
        let cmd = access_register_read_rv64(regno);
        self.jtag
            .rpc(&format!("riscv dmi_write 0x{DMI_COMMAND:x} 0x{cmd:x}"))
            .await?;
        self.wait_abstract_done(regno).await?;
        let lo = self.dmi_read_u32(DMI_DATA0).await?;
        let hi = self.dmi_read_u32(DMI_DATA1).await?;
        Ok(((hi as u64) << 32) | (lo as u64))
    }

    /// Convenience wrapper around [`Self::read_register_via_abstract`]
    /// for GPRs. Indexed by architectural register number (0..31). x0
    /// is hardwired to zero on the architecture but reading it via the
    /// abstract command is still well-defined (returns 0); leaving the
    /// validity check here so callers get a clear error on out-of-
    /// range indices.
    pub async fn read_gpr_via_abstract(&mut self, reg: u8) -> Result<u64, TransportError> {
        if reg >= 32 {
            return Err(TransportError::Protocol(format!(
                "invalid RV GPR index {reg} (must be 0..31)"
            )));
        }
        self.read_register_via_abstract(gpr_abstract_regno(reg))
            .await
    }

    /// Read a 32-bit DMI register via OpenOCD's `riscv dmi_read`. The
    /// reply format is a bare `0x<hex>` token on its own line. Already
    /// the foundation under [`Self::dm_reset`]; lifted to a method here
    /// because the abstract-command path needs to read `abstractcs`,
    /// `data0`, and `data1` repeatedly.
    async fn dmi_read_u32(&mut self, dmi_addr: u32) -> Result<u32, TransportError> {
        let raw = self
            .jtag
            .rpc(&format!("riscv dmi_read 0x{dmi_addr:x}"))
            .await?;
        parse_hex_word(raw.trim()).ok_or_else(|| {
            TransportError::Protocol(format!(
                "dmi_read 0x{dmi_addr:x}: unparseable response `{raw}`"
            ))
        })
    }

    /// Poll `abstractcs` until `busy=0`, then surface a clean error if
    /// `cmderr` is non-zero. Caller passes the regno that was being
    /// accessed for diagnostics. The poll budget is a fixed count
    /// rather than a wall-clock duration so unit tests don't depend on
    /// timer behavior; on healthy hardware `busy` drops in the first
    /// poll.
    async fn wait_abstract_done(&mut self, regno: u16) -> Result<(), TransportError> {
        for _ in 0..ABSTRACTCS_POLL_LIMIT {
            let cs = self.dmi_read_u32(DMI_ABSTRACTCS).await?;
            if cs & ABSTRACTCS_BUSY == 0 {
                let cmderr = (cs & ABSTRACTCS_CMDERR_MASK) >> ABSTRACTCS_CMDERR_SHIFT;
                if cmderr != 0 {
                    return Err(TransportError::Protocol(format!(
                        "abstractcs.cmderr=0x{cmderr:x} reading regno=0x{regno:x} \
                         (abstractcs=0x{cs:08x})"
                    )));
                }
                return Ok(());
            }
        }
        Err(TransportError::Protocol(format!(
            "abstractcs.busy never cleared in {ABSTRACTCS_POLL_LIMIT} polls \
             reading regno=0x{regno:x}"
        )))
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

/// Parse a 32-bit value out of OpenOCD's `riscv dmi_read` reply. Reply is
/// usually a bare `0x12345678`, but tolerate trailing whitespace and the rare
/// `0x...` prefix on its own line.
fn parse_hex_word(s: &str) -> Option<u32> {
    let token = s.split_whitespace().next()?;
    let stripped = token.strip_prefix("0x").unwrap_or(token);
    u32::from_str_radix(stripped, 16).ok()
}
