use async_trait::async_trait;
use heimdall_core::{Artifact, ArtifactKind, DutKind, Observation, State, Stimulus, Verdict};
use heimdall_transport::openocd::OpenocdRpc;
use heimdall_transport::{JtagOps, Transport, TransportKind};
use std::time::Duration;
use tracing::instrument;

use crate::error::DriverError;
use crate::trait_def::{Dut, Result, TestDriver};

pub mod coverage;
pub mod debug_module;
pub mod diff;
pub mod elf_loader;
pub mod observe;

/// Default wall-clock ceiling for a wait-halt. Tuned for real silicon. Slow
/// HDL simulations override per-DUT via [`RiverCpuDriver::with_wait_halt_max`].
const DEFAULT_WAIT_HALT_MAX: Duration = Duration::from_secs(30);

/// Floor for a wait-halt. Even a cycles=0 stimulus must wait long enough for
/// OpenOCD's poll loop to spot a halt.
const WAIT_HALT_MIN: Duration = Duration::from_secs(1);

/// Convert a stimulus budget into a wait-halt timeout. Cycles are treated as
/// milliseconds because we do not yet have a cycles-to-wall-clock model for
/// River silicon. The clamp limits a runaway budget.
fn budget_to_wait_timeout(budget: heimdall_core::StepBudget, max: Duration) -> Duration {
    let d = match budget {
        heimdall_core::StepBudget::Cycles { count } => Duration::from_millis(count),
        heimdall_core::StepBudget::Duration { millis } => Duration::from_millis(millis),
    };
    d.clamp(WAIT_HALT_MIN, max)
}

pub struct RiverCpuDriver<T>
where
    T: Transport + JtagOps + OpenocdRpc + Send + Sync,
{
    target: DutKind,
    pub jtag: T,
    pub uart: Option<Box<dyn Transport + Send + Sync>>,
    pub expect_idcode: Option<u32>,
    wait_halt_max: Duration,
    last_silicon_coverage: Option<coverage::RiverSiliconCoverage>,
    /// ELF entry recorded by the most recent `load`. Used by `run` to seed
    /// `dpc` before resume so the core actually executes the firmware
    /// instead of whatever `dpc` held after the DM reset.
    pending_entry: Option<u64>,
}

impl<T> RiverCpuDriver<T>
where
    T: Transport + JtagOps + OpenocdRpc + Send + Sync,
{
    pub fn new(target: DutKind, jtag: T) -> Self {
        Self {
            target,
            jtag,
            uart: None,
            expect_idcode: None,
            wait_halt_max: DEFAULT_WAIT_HALT_MAX,
            last_silicon_coverage: None,
            pending_entry: None,
        }
    }

    pub fn with_expect_idcode(mut self, idcode: u32) -> Self {
        self.expect_idcode = Some(idcode);
        self
    }

    /// Set the wall-clock ceiling on per-stimulus wait-halt. Slow HDL sim
    /// DUTs need >=120s. Real silicon stays at the default.
    pub fn with_wait_halt_max(mut self, d: Duration) -> Self {
        self.wait_halt_max = d;
        self
    }
}

#[async_trait]
impl<T> TestDriver for RiverCpuDriver<T>
where
    T: Transport + JtagOps + OpenocdRpc + Send + Sync,
{
    fn target(&self) -> DutKind {
        self.target
    }
    fn required_transports(&self) -> &[TransportKind] {
        const REQ: &[TransportKind] = &[TransportKind::Jtag];
        REQ
    }

    #[instrument(skip(self, _dut))]
    async fn prepare(&mut self, _dut: &mut Dut) -> Result<()> {
        self.jtag.open().await?;
        {
            let mut dm = debug_module::DebugModule::new(&mut self.jtag);
            dm.dm_reset().await?;
        }
        let chain = self.jtag.scan_idcode().await?;
        if let (Some(expected), Some(got)) = (self.expect_idcode, chain.first().copied()) {
            if got != expected {
                return Err(DriverError::IdcodeMismatch { got, expected });
            }
        }
        Ok(())
    }

    async fn compile(
        &mut self,
        input: &Artifact,
        tools: &heimdall_tools::ToolChain,
    ) -> Result<Artifact> {
        let target = heimdall_tools::TargetSpec {
            dut_kind: self.target,
            desired_output: ArtifactKind::ElfRiscv,
        };
        let out = tools
            .build(input.clone(), &target, &heimdall_tools::ToolOpts::default())
            .await?;
        Ok(out)
    }

    #[instrument(skip(self, _dut, image))]
    async fn load(&mut self, _dut: &mut Dut, image: &Artifact) -> Result<()> {
        // Parse the ELF and walk every PT_LOAD segment to its physical
        // load address. The previous implementation wrote the raw ELF
        // file bytes to a hardcoded 0x80000000, so the bring-up firmware
        // never sat at its linked address and the core ran through empty
        // memory at resume.
        let parsed = elf_loader::parse(&image.bytes)?;
        let mut dm = debug_module::DebugModule::new(&mut self.jtag);
        dm.halt().await?;
        for seg in &parsed.segments {
            if !seg.bytes.is_empty() {
                dm.write_mem(seg.paddr, &seg.bytes).await?;
            }
            // BSS tail: write zeros so the core sees a clean region. Cheap
            // in absolute bytes since bring-up firmware is tiny.
            if seg.zero_tail > 0 {
                let zeros = vec![0u8; seg.zero_tail as usize];
                let tail_addr = seg.paddr + seg.bytes.len() as u64;
                dm.write_mem(tail_addr, &zeros).await?;
            }
        }
        self.pending_entry = Some(parsed.entry);
        Ok(())
    }

    async fn run(&mut self, _dut: &mut Dut, stim: &Stimulus) -> Result<Observation> {
        let timeout = budget_to_wait_timeout(stim.budget, self.wait_halt_max);
        let started = std::time::Instant::now();
        let entry = self.pending_entry.ok_or(DriverError::State(
            "river run() called before load(); pending_entry unset",
        ))?;

        {
            let mut dm = debug_module::DebugModule::new(&mut self.jtag);
            // Seed dpc so resume jumps to the ELF entry instead of running
            // from whatever pc the DM reset left behind (~0, parks at 0x388
            // running through empty memory).
            dm.write_reg("dpc", entry).await?;
            dm.resume().await?;
            if let Err(e) = dm.wait_halt(timeout).await {
                // Program never halted within the budget. Force-halt the
                // CPU so the next iteration's load+resume isn't fighting
                // a still-running program (and rbb sockets don't get
                // wedged by a runaway). A halt failure here is logged
                // and swallowed. The original wait_halt error is the
                // one the caller needs to see.
                if let Err(halt_err) = dm.halt().await {
                    tracing::warn!(
                        error = %halt_err,
                        "force-halt after wait_halt timeout failed; \
                         subsequent iterations may be wedged"
                    );
                }
                return Err(e.into());
            }
        }

        let state = observe::snapshot_xregs_pc(&mut self.jtag).await?;
        Ok(Observation::new(state, started.elapsed()))
    }

    async fn observe(&mut self, _dut: &mut Dut) -> Result<State> {
        let state = observe::snapshot_xregs_pc(&mut self.jtag)
            .await
            .map_err(DriverError::from)?;
        if let Some(heimdall_core::ValueRepr::U64(pc)) = state.fields.get("pc") {
            self.last_silicon_coverage = Some(coverage::RiverSiliconCoverage::from_pc(*pc));
        }
        Ok(state)
    }

    fn coverage(&self) -> Option<&dyn heimdall_golden::CoverageSource> {
        self.last_silicon_coverage
            .as_ref()
            .map(|c| c as &dyn heimdall_golden::CoverageSource)
    }

    async fn diff(&self, dut_state: &State, golden_state: &State) -> Verdict {
        diff::diff_states(dut_state, golden_state)
    }

    async fn release(&mut self, _dut: &mut Dut) -> Result<()> {
        self.jtag.close().await?;
        Ok(())
    }

    async fn probe_isa(&mut self) -> Result<Option<crate::trait_def::IsaProbe>> {
        // Halt the core so the abstract-command read of CSR misa is
        // well-defined. The defensive halt + short wait_halt mirror
        // `observe::snapshot_xregs_pc`'s prelude on broken openocd
        // builds (the same 0.12.0 cache-staleness issue). Probing
        // from a running core is implementation-defined and usually
        // returns cmderr=halt/resume.
        let mut dm = debug_module::DebugModule::new(&mut self.jtag);
        dm.halt().await.map_err(DriverError::from)?;
        dm.wait_halt(Duration::from_millis(500))
            .await
            .map_err(DriverError::from)?;
        // CSR 0x301 = misa. The abstract command returns the full
        // 64-bit XLEN-extended value on RV64 and the low 32 bits on
        // RV32. The MXL field tells us which.
        const CSR_MISA: u16 = 0x301;
        let raw = dm
            .read_register_via_abstract(CSR_MISA)
            .await
            .map_err(DriverError::from)?;
        Ok(Some(parse_misa(raw)))
    }
}

/// Decode CSR `misa` (RISC-V Privileged Spec). Layout:
/// - `MXL` (bits XLEN-1:XLEN-2): 1=RV32, 2=RV64, 3=RV128.
/// - Bit 0..25: extension bitmap, bit i set means extension `'a' + i`
///   is implemented.
///
/// We read a 64-bit value over the wire (the DMI abstract-command
/// path) regardless of actual XLEN. On RV64 cores MXL is at bits
/// 62:63. On RV32 cores the upper bits are typically zero and MXL
/// sits at bits 30:31 of the low 32-bit half. We probe both
/// positions and prefer whichever encodes a known value.
pub fn parse_misa(value: u64) -> crate::trait_def::IsaProbe {
    let low32 = value as u32;
    let mxl_rv64 = ((value >> 62) & 0x3) as u8;
    let mxl_rv32 = ((low32 >> 30) & 0x3) as u8;
    let xlen = if mxl_rv64 == 2 {
        64
    } else if mxl_rv32 == 1 {
        32
    } else {
        // Fall back to RV32 when neither MXL field decodes cleanly.
        // RV32 is the safer default since RV32I is the universal base.
        32
    };
    let mut extensions = Vec::new();
    for i in 0..26u32 {
        if (low32 & (1u32 << i)) != 0 {
            let letter = (b'a' + i as u8) as char;
            extensions.push(letter.to_string());
        }
    }
    crate::trait_def::IsaProbe { xlen, extensions }
}

#[cfg(test)]
mod misa_tests {
    use super::*;

    #[test]
    fn parse_misa_rv64imac() {
        // MXL=2 at bits 62:63 (RV64), extensions I+M+A+C.
        let i = 1u64 << 8;
        let m = 1u64 << 12;
        let a = 1u64 << 0;
        let c = 1u64 << 2;
        let mxl = 2u64 << 62;
        let raw = mxl | i | m | a | c;
        let probe = parse_misa(raw);
        assert_eq!(probe.xlen, 64);
        assert!(probe.extensions.contains(&"i".to_string()));
        assert!(probe.extensions.contains(&"m".to_string()));
        assert!(probe.extensions.contains(&"a".to_string()));
        assert!(probe.extensions.contains(&"c".to_string()));
    }

    #[test]
    fn parse_misa_rv32i_minimal() {
        // MXL=1 at bits 30:31 (RV32), only I set.
        let mxl: u32 = 1 << 30;
        let i: u32 = 1 << 8;
        let raw = (mxl | i) as u64;
        let probe = parse_misa(raw);
        assert_eq!(probe.xlen, 32);
        assert_eq!(probe.extensions, vec!["i".to_string()]);
    }

    #[test]
    fn parse_misa_unknown_mxl_defaults_to_rv32() {
        // Neither bit position holds a recognised MXL value.
        let probe = parse_misa(0);
        assert_eq!(probe.xlen, 32);
        assert!(probe.extensions.is_empty());
    }
}
