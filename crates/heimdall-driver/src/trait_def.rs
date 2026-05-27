use async_trait::async_trait;
use heimdall_core::{Artifact, DutId, DutKind, Observation, State, Stimulus, Verdict};
use heimdall_transport::TransportKind;

use crate::error::DriverError;

pub type Result<T> = std::result::Result<T, DriverError>;

/// A Dut is a handle to a physical DUT plus the transports the driver
/// has been handed for it. Concrete transport types live behind dyn pointers
/// in the driver impl; this struct is just identity + metadata.
pub struct Dut {
    pub id: DutId,
    pub kind: DutKind,
}

impl Dut {
    pub fn new(id: DutId, kind: DutKind) -> Self {
        Self { id, kind }
    }
}

#[async_trait]
pub trait TestDriver: Send + Sync {
    fn target(&self) -> DutKind;
    fn required_transports(&self) -> &[TransportKind];

    async fn prepare(&mut self, dut: &mut Dut) -> Result<()>;
    async fn compile(
        &mut self,
        input: &Artifact,
        tools: &heimdall_tools::ToolChain,
    ) -> Result<Artifact>;
    async fn load(&mut self, dut: &mut Dut, image: &Artifact) -> Result<()>;
    async fn run(&mut self, dut: &mut Dut, stimulus: &Stimulus) -> Result<Observation>;
    async fn observe(&mut self, dut: &mut Dut) -> Result<State>;
    async fn diff(&self, dut_state: &State, golden_state: &State) -> Verdict;
    async fn release(&mut self, dut: &mut Dut) -> Result<()>;

    /// Silicon-side coverage from the most recent run/observe. Default
    /// returns None; drivers that can extract a coverage signal (e.g., PC
    /// trace via debug module) override this.
    fn coverage(&self) -> Option<&dyn heimdall_golden::CoverageSource> {
        None
    }
}
