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

        let mut cmd = Command::new(&self.binary);
        cmd.arg("--log-commits")
            .arg(format!("--log={}", log_path.display()))
            .arg(format!("--max-cycles={max_cycles}"));
        for a in &self.extra_args {
            cmd.arg(a);
        }
        cmd.arg(&img_path)
            .stdin(Stdio::null())
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
