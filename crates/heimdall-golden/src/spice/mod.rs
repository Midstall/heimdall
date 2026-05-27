//! ngspice-backed GoldenModel. Generates a stimulus .sp file at each step,
//! invokes ngspice in batch mode, parses the .raw ASCII output for watched
//! node voltages, and returns digital state + activity coverage.

mod parse;
pub mod render;

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::Stdio;

use async_trait::async_trait;
use heimdall_core::{Artifact, DutKind, State, StepBudget, ValueRepr};
use tokio::process::Command;
use tracing::{debug, instrument};

use crate::error::GoldenError;
use crate::trait_def::{CoverageSource, GoldenModel, Result, StepOutcome};

pub use parse::{RawTrace, parse_raw_ascii};

/// Configuration for a SPICE node watched by the golden.
#[derive(Debug, Clone)]
pub struct SpiceWatch {
    /// heimdall-side name (e.g., "io_2").
    pub name: String,
    /// SPICE node name (e.g., "n_pad_out_2").
    pub spice_node: String,
    /// Direction. Inputs are driven from Stimulus.inputs, outputs are observed.
    pub direction: SpiceDir,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpiceDir {
    In,
    Out,
}

/// SpiceCoverage: one bit per watched output node. Bit set if the node's
/// voltage range during the run exceeded `activity_threshold_v`.
pub struct SpiceCoverage {
    bits: Vec<u8>,
}

impl SpiceCoverage {
    pub fn buckets() -> usize {
        256
    }

    /// Construct from a raw bit-vector. Useful for tests and for callers
    /// that have coverage data from a source other than ngspice.
    pub fn from_bits_for_test(bits: Vec<u8>) -> Self {
        Self { bits }
    }

    pub fn from_traces(traces: &[RawTrace], outputs: &[SpiceWatch], threshold_v: f64) -> Self {
        let mut bits = vec![0u8; Self::buckets()];
        for (idx, w) in outputs.iter().enumerate() {
            if let Some(trace) = traces.iter().find(|t| t.matches_node(&w.spice_node)) {
                let span = trace.range();
                if span > threshold_v && idx < Self::buckets() * 8 {
                    let byte = idx / 8;
                    let bit = (idx % 8) as u8;
                    bits[byte] |= 1 << bit;
                }
            }
        }
        Self { bits }
    }
}

impl CoverageSource for SpiceCoverage {
    fn snapshot(&self) -> Vec<u8> {
        self.bits.clone()
    }
}

pub struct SpiceGoldenModel {
    /// Path to the ngspice binary (or just "ngspice" to use PATH).
    pub binary: PathBuf,
    /// Path to the device netlist (.sp/.cir) the user supplies.
    pub device_netlist: PathBuf,
    /// Watched nodes (inputs are driven, outputs are observed).
    pub watches: Vec<SpiceWatch>,
    /// Supply voltage for digital interpretation (V > vdd/2 => true).
    pub vdd: f64,
    /// Transient analysis settings.
    pub tstep: f64,
    pub tstop: f64,
    /// Activity threshold for SpiceCoverage.
    pub activity_threshold_v: f64,
    /// DUT kind this model targets (informational).
    target: DutKind,

    /// Cached state after the last step.
    last_traces: Option<Vec<RawTrace>>,
    last_coverage: Option<SpiceCoverage>,
    last_inputs: BTreeMap<String, bool>,
}

impl SpiceGoldenModel {
    pub fn new(target: DutKind, device_netlist: impl Into<PathBuf>) -> Self {
        Self {
            binary: PathBuf::from("ngspice"),
            device_netlist: device_netlist.into(),
            watches: Vec::new(),
            vdd: 1.8,
            tstep: 1e-9,
            tstop: 100e-9,
            activity_threshold_v: 0.5,
            target,
            last_traces: None,
            last_coverage: None,
            last_inputs: BTreeMap::new(),
        }
    }

    pub fn with_binary(mut self, path: impl Into<PathBuf>) -> Self {
        self.binary = path.into();
        self
    }

    pub fn with_watches(mut self, watches: Vec<SpiceWatch>) -> Self {
        self.watches = watches;
        self
    }

    pub fn with_vdd(mut self, vdd: f64) -> Self {
        self.vdd = vdd;
        self
    }

    pub fn with_transient(mut self, tstep: f64, tstop: f64) -> Self {
        self.tstep = tstep;
        self.tstop = tstop;
        self
    }

    /// Set the input values to drive on the next step.
    pub fn set_inputs(&mut self, inputs: BTreeMap<String, bool>) {
        self.last_inputs = inputs;
    }

    fn generate_sp(&self, out_path: &std::path::Path) -> String {
        let mut s = String::new();
        s.push_str("* heimdall SPICE fuzz step (auto-generated)\n");
        s.push_str(".title heimdall fuzz step\n");
        // Include the user's device netlist verbatim.
        s.push_str(&format!(".include \"{}\"\n", self.device_netlist.display()));
        // Drive inputs with DC sources.
        for w in &self.watches {
            if matches!(w.direction, SpiceDir::In) {
                let v = if *self.last_inputs.get(&w.name).unwrap_or(&false) {
                    self.vdd
                } else {
                    0.0
                };
                s.push_str(&format!(
                    "V_heimdall_{} {} 0 DC {}\n",
                    w.spice_node, w.spice_node, v
                ));
            }
        }
        // ngspice quirks:
        //   - The `.options` circuit-deck directive doesn't accept filetype;
        //     `set filetype=ascii` must live in a .control block.
        //   - `set rawfile=<path>` lowercases its argument internally, so the
        //     caller MUST pass a lowercase path. step() guarantees this by
        //     constructing paths via `format!("heimdall-spice-{pid}-{nanos}.raw")`
        //     which contains only lowercase chars + digits.
        s.push_str(".control\n");
        s.push_str("set filetype=ascii\n");
        s.push_str(&format!("set rawfile={}\n", out_path.display()));
        s.push_str(&format!("tran {} {}\n", self.tstep, self.tstop));
        s.push_str("write\n");
        s.push_str(".endc\n");
        s.push_str(".end\n");
        s
    }
}

#[async_trait]
impl GoldenModel for SpiceGoldenModel {
    fn target(&self) -> DutKind {
        self.target
    }

    async fn reset(&mut self) -> Result<()> {
        self.last_traces = None;
        self.last_coverage = None;
        Ok(())
    }

    async fn load(&mut self, _image: &Artifact) -> Result<()> {
        // SPICE flows don't have a "load" step in the spike-style sense.
        // The netlist already describes the device. Treat as a no-op so the
        // Runner's load->step->observe flow still works.
        Ok(())
    }

    #[instrument(skip(self))]
    async fn step(&mut self, budget: StepBudget) -> Result<StepOutcome> {
        let _ = budget; // budget influences nothing in v1 (.tran params come from the model)

        // ngspice 45's `set rawfile=<path>` LOWERCASES the entire path,
        // including the directory. nix-shell's TMPDIR ($TMPDIR =
        // /tmp/nix-shell.yssobG) often contains uppercase letters, so a
        // path produced via std::env::temp_dir() can't be passed to ngspice
        // verbatim. Use a fixed all-lowercase scratch dir under /tmp/heimdall.
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let tmp_dir = std::path::PathBuf::from("/tmp/heimdall-spice");
        std::fs::create_dir_all(&tmp_dir)?;
        let raw_path = tmp_dir.join(format!("run-{pid}-{nanos}.raw"));
        let sp_path = tmp_dir.join(format!("run-{pid}-{nanos}.sp"));

        // Write the generated .sp (raw_path embedded via `set rawfile=...`).
        let sp_contents = self.generate_sp(&raw_path);
        {
            let mut f = std::fs::File::create(&sp_path)?;
            f.write_all(sp_contents.as_bytes())?;
        }

        let output = Command::new(&self.binary)
            .arg("-b")
            .arg(&sp_path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await?;
        let _ = std::fs::remove_file(&sp_path);
        if !output.status.success() {
            return Err(GoldenError::SpikeBadExit {
                status: output.status.code().unwrap_or(-1),
                stderr: format!("ngspice: {}", String::from_utf8_lossy(&output.stderr)),
            });
        }

        let raw_text = tokio::fs::read_to_string(&raw_path).await.map_err(|e| {
            GoldenError::ParseSpike(format!(
                "could not read {} after ngspice exited; stdout={:?} stderr={:?} err={}",
                raw_path.display(),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
                e
            ))
        })?;
        let traces = parse_raw_ascii(&raw_text)
            .map_err(|e| GoldenError::ParseSpike(format!("ngspice .raw: {e}")))?;
        debug!(n_traces = traces.len(), "parsed ngspice .raw");

        // Clean up the raw file. sp_file drops automatically.
        let _ = std::fs::remove_file(&raw_path);

        let outputs: Vec<SpiceWatch> = self
            .watches
            .iter()
            .filter(|w| matches!(w.direction, SpiceDir::Out))
            .cloned()
            .collect();
        self.last_coverage = Some(SpiceCoverage::from_traces(
            &traces,
            &outputs,
            self.activity_threshold_v,
        ));
        self.last_traces = Some(traces);

        Ok(StepOutcome::RanFully)
    }

    async fn observe(&mut self) -> Result<State> {
        let traces = self.last_traces.as_ref().ok_or(GoldenError::NotLoaded)?;
        let mut state = State::new();
        let threshold = self.vdd / 2.0;
        for w in &self.watches {
            if matches!(w.direction, SpiceDir::Out) {
                let last_v = traces
                    .iter()
                    .find(|t| t.matches_node(&w.spice_node))
                    .map(|t| t.last_value())
                    .unwrap_or(0.0);
                state = state.with(w.name.clone(), ValueRepr::Bool(last_v > threshold));
            }
        }
        Ok(state)
    }

    fn coverage(&self) -> Option<&dyn CoverageSource> {
        self.last_coverage
            .as_ref()
            .map(|c| c as &dyn CoverageSource)
    }
}
