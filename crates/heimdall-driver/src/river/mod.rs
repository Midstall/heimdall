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
pub mod observe;

/// Convert a stimulus budget into a wait-halt timeout. Cycles are treated as
/// milliseconds with a sane floor and ceiling because we don't yet have a real
/// cycles-to-wall-clock model for River silicon. Once that lands this becomes
/// a per-DUT calibration.
fn budget_to_wait_timeout(budget: heimdall_core::StepBudget) -> Duration {
    const MIN: Duration = Duration::from_secs(1);
    const MAX: Duration = Duration::from_secs(30);
    let d = match budget {
        heimdall_core::StepBudget::Cycles { count } => Duration::from_millis(count),
        heimdall_core::StepBudget::Duration { millis } => Duration::from_millis(millis),
    };
    d.clamp(MIN, MAX)
}

pub struct RiverCpuDriver<T>
where
    T: Transport + JtagOps + OpenocdRpc + Send + Sync,
{
    target: DutKind,
    pub jtag: T,
    pub uart: Option<Box<dyn Transport + Send + Sync>>,
    pub expect_idcode: Option<u32>,
    last_silicon_coverage: Option<coverage::RiverSiliconCoverage>,
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
            last_silicon_coverage: None,
        }
    }

    pub fn with_expect_idcode(mut self, idcode: u32) -> Self {
        self.expect_idcode = Some(idcode);
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
        self.jtag
            .reset(heimdall_transport::ResetTarget::DebugModule)
            .await?;
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
        let mut dm = debug_module::DebugModule::new(&mut self.jtag);
        dm.halt().await?;
        dm.write_mem(0x8000_0000, &image.bytes).await?;
        Ok(())
    }

    async fn run(&mut self, _dut: &mut Dut, stim: &Stimulus) -> Result<Observation> {
        let timeout = budget_to_wait_timeout(stim.budget);
        let started = std::time::Instant::now();

        {
            let mut dm = debug_module::DebugModule::new(&mut self.jtag);
            dm.resume().await?;
            dm.wait_halt(timeout).await?;
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
}
