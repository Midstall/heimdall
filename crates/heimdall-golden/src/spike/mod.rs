use async_trait::async_trait;
use heimdall_core::{Artifact, DutKind, State, StepBudget};
use std::path::PathBuf;
use std::process::Stdio;
use tempfile::NamedTempFile;
use tokio::process::Command;
use tracing::instrument;

use crate::error::GoldenError;
use crate::trait_def::{CoverageSource, GoldenModel, Result, StepOutcome};

mod log_parser;
use log_parser::parse_final_state;

/// Coverage bitmap built from a spike commit log. Each PC is hashed into a
/// fixed-size bitmap by right-shifting two bits (instruction-aligned) and
/// masking into the bitmap index space.
pub struct SpikeCoverage {
    bits: Vec<u8>,
}

impl SpikeCoverage {
    pub fn buckets() -> usize {
        1024
    }

    pub fn from_log(log: &str) -> Self {
        let mut bits = vec![0u8; Self::buckets()];
        for line in log.lines() {
            // commit lines: "core N   N: 3 0x<pc> (0x<insn>)"
            let Some(rest) = line.split_once(':').map(|x| x.1.trim()) else {
                continue;
            };
            let mut toks = rest.split_whitespace();
            let Some(_priv) = toks.next() else { continue };
            let Some(pc_tok) = toks.next() else { continue };
            if let Some(stripped) = pc_tok.strip_prefix("0x") {
                if let Ok(pc) = u64::from_str_radix(stripped, 16) {
                    let idx = ((pc >> 2) as usize) & (Self::buckets() * 8 - 1);
                    let byte = idx / 8;
                    let bit = (idx % 8) as u8;
                    bits[byte] |= 1 << bit;
                }
            }
        }
        Self { bits }
    }
}

impl CoverageSource for SpikeCoverage {
    fn snapshot(&self) -> Vec<u8> {
        self.bits.clone()
    }
}

/// One-shot spike runner. Each call to `step` invokes spike with the loaded
/// ELF, captures `--log-commits` to a temp file, parses out the final state on
/// `observe`. Stateless across runs.
pub struct SpikeOneShot {
    binary: PathBuf,
    extra_args: Vec<String>,
    target: DutKind,
    image: Option<Artifact>,
    last_log: Option<String>,
    last_coverage: Option<SpikeCoverage>,
}

impl SpikeOneShot {
    pub fn new(binary: impl Into<PathBuf>, target: DutKind) -> Self {
        Self {
            binary: binary.into(),
            extra_args: Vec::new(),
            target,
            image: None,
            last_log: None,
            last_coverage: None,
        }
    }

    pub fn with_extra_args(mut self, args: impl IntoIterator<Item = String>) -> Self {
        self.extra_args.extend(args);
        self
    }
}

#[async_trait]
impl GoldenModel for SpikeOneShot {
    fn target(&self) -> DutKind {
        self.target
    }

    async fn reset(&mut self) -> Result<()> {
        self.image = None;
        self.last_log = None;
        self.last_coverage = None;
        Ok(())
    }

    async fn load(&mut self, image: &Artifact) -> Result<()> {
        self.image = Some(image.clone());
        Ok(())
    }

    #[instrument(skip(self, budget))]
    async fn step(&mut self, budget: StepBudget) -> Result<StepOutcome> {
        let image = self.image.as_ref().ok_or(GoldenError::NotLoaded)?;
        let mut img_file = NamedTempFile::new()?;
        use std::io::Write as _;
        img_file.write_all(&image.bytes)?;
        let img_path = img_file.into_temp_path();

        let log_file = NamedTempFile::new()?;
        let log_path = log_file.into_temp_path();

        let max_cycles = match budget {
            StepBudget::Cycles { count } => count,
            _ => return Err(GoldenError::UnsupportedBudget(budget, "spike-one-shot")),
        };

        // Spike's `--max-cycles` only exists on some forks; the upstream
        // / nixpkgs build at this path doesn't accept it. Spike's `-d`
        // mode + `--debug-cmd=<file>` IS universal: write a script and
        // spike reads it deterministically (piping to stdin doesn't work
        // reliably on this build, spike spam-loops its prompt instead of
        // reading EOF). The `--log-commits` log file is populated the
        // same way.
        //
        // Spike's bootrom at 0x1000 seeds t0=entry, a0=mhartid (0),
        // a1=dtb_ptr (0x1020) before jumping to user code. Real silicon
        // / the River emulator enter user code with all-zero GPRs.
        // Aligning the initial GPR state across both models is done by
        // a register-zeroing prologue baked into the generated programs
        // themselves (`heimdall_fuzzer::RawAsmGen::zero_init_prologue`),
        // not via spike-side `reg` writes (which are read-only in
        // spike's interactive debug console).
        let entry = parse_elf_entry(&image.bytes).unwrap_or(0);
        let script = build_debug_cmd_script(entry, max_cycles);
        let mut cmd_file = NamedTempFile::new()?;
        cmd_file.write_all(script.as_bytes())?;
        let cmd_path = cmd_file.into_temp_path();

        let mut cmd = Command::new(&self.binary);
        cmd.arg("-d")
            .arg(format!("--debug-cmd={}", cmd_path.display()))
            .arg("--log-commits")
            .arg(format!("--log={}", log_path.display()));
        // Spike's default memory layout is 256 MiB of DRAM at 0x80000000.
        // The fuzz path's `RawBytesToElfRiscv` wraps programs at 0x10000,
        // and the bring-up firmware also links to 0x10000, so without
        // adding a memory region Spike refuses to load the ELF. Derive
        // the needed `-m` args from the ELF's PT_LOAD segments unless
        // the caller already supplied a custom layout via `extra_args`.
        let user_provided_mem = self
            .extra_args
            .iter()
            .any(|a| a == "-m" || a.starts_with("-m") || a.starts_with("--mem"));
        if !user_provided_mem {
            for region in derive_mem_regions(&image.bytes) {
                cmd.arg(format!("-m{:#x}:{}", region.base, region.size));
            }
        }
        for a in &self.extra_args {
            cmd.arg(a);
        }
        cmd.arg(&img_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let output = cmd.output().await?;
        if !output.status.success() {
            return Err(GoldenError::SpikeBadExit {
                status: output.status.code().unwrap_or(-1),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }

        let log = tokio::fs::read_to_string(&log_path).await?;
        self.last_coverage = Some(SpikeCoverage::from_log(&log));
        self.last_log = Some(log);
        Ok(StepOutcome::RanFully)
    }

    async fn observe(&mut self) -> Result<State> {
        let log = self.last_log.as_ref().ok_or(GoldenError::NotLoaded)?;
        parse_final_state(log)
    }

    fn coverage(&self) -> Option<&dyn CoverageSource> {
        self.last_coverage
            .as_ref()
            .map(|c| c as &dyn CoverageSource)
    }
}

/// Parse the ELF's `e_entry` so the spike script knows where the program
/// starts after the bootrom. Returns `None` on any parse failure, which
/// is handled upstream by falling back to `0` (which spike's `until pc 0`
/// will treat as "run forever" so the script then just runs from the
/// bootrom; lossy but doesn't break the fuzz).
fn parse_elf_entry(elf_bytes: &[u8]) -> Option<u64> {
    use elf::ElfBytes;
    use elf::endian::AnyEndian;
    let file = ElfBytes::<AnyEndian>::minimal_parse(elf_bytes).ok()?;
    Some(file.ehdr.e_entry)
}

/// Build the `--debug-cmd` script that spike consumes. Three phases:
/// 1. `until pc 0 <entry>` runs the bootrom up to the program entry so
///    the bounded `r` budget below covers user code, not the bootrom.
/// 2. `r <max_cycles>` runs the program proper. The trailing ebreak
///    that `RawBytesToElfRiscv`-wrapped fuzz programs carry halts spike
///    well before `max_cycles`. Bring-up firmware's own `ebreak` does
///    the same.
/// 3. `q` exits cleanly.
///
/// Note we do NOT issue `reg 0 xN <val>` writes here. Spike's
/// interactive console only READS registers, and the 3-arg form errors
/// silently ("Bad or missing arguments"). The DUT-aligned all-zero
/// initial-GPR state is established by an in-program prologue baked
/// into the fuzz programs themselves (see
/// `heimdall_fuzzer::RawAsmGen::zero_init_prologue`), which runs
/// symmetrically on spike and the DUT.
fn build_debug_cmd_script(entry: u64, max_cycles: u64) -> String {
    let mut s = String::with_capacity(64);
    if entry != 0 {
        s.push_str(&format!("until pc 0 {entry:#x}\n"));
    }
    s.push_str(&format!("r {max_cycles}\n"));
    s.push_str("q\n");
    s
}

/// One contiguous memory region passed to Spike via `-m<base>:<size>`.
/// `base` is page-aligned down, `size` is page-aligned up; Spike requires
/// 4 KiB alignment on both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MemRegion {
    base: u64,
    size: u64,
}

const PAGE: u64 = 0x1000;
/// Lower-bound a generated region at 1 MiB so a tiny PT_LOAD doesn't get
/// a 4 KiB region that immediately overflows into the next instruction
/// fetch. Cheap; Spike sparsely allocates.
const MIN_REGION_SIZE: u64 = 1 << 20;

/// Walk the ELF's PT_LOAD segments and return one `-m`-ready region per
/// segment. Returns an empty Vec on parse failure so the spike invocation
/// falls through to its default 0x80000000 region. Non-fatal: a misparse
/// produces a Spike-side load failure that the caller surfaces.
fn derive_mem_regions(elf_bytes: &[u8]) -> Vec<MemRegion> {
    use elf::ElfBytes;
    use elf::abi::PT_LOAD;
    use elf::endian::AnyEndian;
    let Ok(file) = ElfBytes::<AnyEndian>::minimal_parse(elf_bytes) else {
        return Vec::new();
    };
    let Some(phdrs) = file.segments() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for ph in phdrs {
        if ph.p_type != PT_LOAD {
            continue;
        }
        let span = ph.p_memsz.max(ph.p_filesz);
        if span == 0 {
            continue;
        }
        let base = ph.p_paddr & !(PAGE - 1);
        let raw_size = (ph.p_paddr - base) + span;
        let mut size = raw_size.next_multiple_of(PAGE);
        if size < MIN_REGION_SIZE {
            size = MIN_REGION_SIZE;
        }
        out.push(MemRegion { base, size });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reuse the elf_loader-style hand-built RV64 ELF: one PT_LOAD of 4
    /// bytes at paddr 0x10000, entry 0x100b0.
    fn minimal_rv64_elf() -> Vec<u8> {
        let mut buf = vec![0u8; 0x40 + 0x38 + 4];
        buf[0..4].copy_from_slice(b"\x7fELF");
        buf[4] = 2;
        buf[5] = 1;
        buf[6] = 1;
        buf[16..18].copy_from_slice(&2u16.to_le_bytes());
        buf[18..20].copy_from_slice(&243u16.to_le_bytes());
        buf[20..24].copy_from_slice(&1u32.to_le_bytes());
        buf[24..32].copy_from_slice(&0x100b0u64.to_le_bytes());
        buf[32..40].copy_from_slice(&0x40u64.to_le_bytes());
        buf[52..54].copy_from_slice(&0x40u16.to_le_bytes());
        buf[54..56].copy_from_slice(&0x38u16.to_le_bytes());
        buf[56..58].copy_from_slice(&1u16.to_le_bytes());
        let ph = 0x40;
        buf[ph..ph + 4].copy_from_slice(&1u32.to_le_bytes());
        buf[ph + 4..ph + 8].copy_from_slice(&5u32.to_le_bytes());
        buf[ph + 8..ph + 16].copy_from_slice(&0x78u64.to_le_bytes());
        buf[ph + 16..ph + 24].copy_from_slice(&0x10000u64.to_le_bytes());
        buf[ph + 24..ph + 32].copy_from_slice(&0x10000u64.to_le_bytes());
        buf[ph + 32..ph + 40].copy_from_slice(&4u64.to_le_bytes());
        buf[ph + 40..ph + 48].copy_from_slice(&4u64.to_le_bytes());
        buf[ph + 48..ph + 56].copy_from_slice(&0x1000u64.to_le_bytes());
        buf
    }

    #[test]
    fn derives_one_region_at_page_aligned_base() {
        let elf = minimal_rv64_elf();
        let regions = derive_mem_regions(&elf);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].base, 0x10000);
        // Minimum 1 MiB region so tiny segments don't get a single page.
        assert_eq!(regions[0].size, MIN_REGION_SIZE);
    }

    #[test]
    fn non_elf_bytes_return_no_regions() {
        let regions = derive_mem_regions(b"not an elf");
        assert!(regions.is_empty());
    }

    #[test]
    fn debug_cmd_script_walks_to_entry_then_runs() {
        let script = build_debug_cmd_script(0x100b0, 1000);
        // Step 1: walk past the bootrom to user code.
        assert!(script.starts_with("until pc 0 0x100b0\n"));
        // Step 2: bounded run.
        assert!(script.contains("\nr 1000\n"));
        // Step 3: clean exit.
        assert!(script.trim_end().ends_with("q"));
        // Crucially: spike's interactive `reg <core> <reg> <val>` is
        // READ-ONLY in current builds; relying on it silently no-ops.
        // The in-program prologue baked by `RawAsmGen::zero_init_prologue`
        // handles initial-state alignment instead.
        assert!(
            !script.contains("reg 0 x"),
            "reg-write lines must not appear (spike reg is read-only): {script}",
        );
    }

    #[test]
    fn debug_cmd_script_skips_until_when_entry_unknown() {
        // entry=0 means we couldn't parse the ELF; fall back to running
        // straight from the bootrom (no `until` line so spike doesn't
        // hang on an unreachable pc).
        let script = build_debug_cmd_script(0, 100);
        assert!(!script.contains("until "), "got: {script}");
        assert!(script.starts_with("r 100\n"));
        assert!(script.trim_end().ends_with("q"));
    }

    #[test]
    fn elf_entry_parse_returns_ehdr_e_entry() {
        // Use the existing minimal ELF builder.
        let elf = minimal_rv64_elf();
        assert_eq!(parse_elf_entry(&elf), Some(0x100b0));
    }
}
