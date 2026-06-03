use async_trait::async_trait;
use heimdall_core::{Artifact, DutId, DutKind, Observation, State, Stimulus, Verdict};
use heimdall_transport::TransportKind;

use crate::error::DriverError;

pub type Result<T> = std::result::Result<T, DriverError>;

/// Result of a JTAG-side ISA probe (e.g. reading CSR misa via DMI).
/// Strings here are canonical extension identifiers: single misa
/// letters (`"i"`, `"m"`, ...) or Z-style names (`"zicsr"`,
/// `"zifencei"`) so downstream callers (the daemon's fuzz factory)
/// don't need to depend on the fuzzer's `RvExtension` enum to
/// consume probe results.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsaProbe {
    /// XLEN in bits (32 or 64). Derived from `misa.MXL`.
    pub xlen: u8,
    /// Extension identifiers as misa letters or Z-style names.
    pub extensions: Vec<String>,
}

/// A Dut is a handle to a physical DUT plus the transports the driver
/// has been handed for it. Concrete transport types live behind dyn pointers
/// in the driver impl. This struct is just identity + metadata.
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
    /// returns None. Drivers that can extract a coverage signal (e.g., PC
    /// trace via debug module) override this.
    fn coverage(&self) -> Option<&dyn heimdall_golden::CoverageSource> {
        None
    }

    /// Probe the DUT's architectural ISA capabilities. For RISC-V
    /// drivers this typically reads the `misa` CSR via the debug
    /// module's abstract-command interface, decodes `MXL` for XLEN
    /// and the lower 26 bits as the extension bitmap, and returns
    /// `Some(IsaProbe)`. Implementations that can't determine the
    /// ISA (mock drivers, non-CPU drivers, transports that can't
    /// reach a CSR) return `Ok(None)`. The default returns `Ok(None)`
    /// so existing impls don't need to change.
    ///
    /// The caller is responsible for ensuring the driver is in a
    /// state where the probe can execute (transport open, debug
    /// module reachable). In practice that's "after `prepare()` has
    /// run at least once."
    async fn probe_isa(&mut self) -> Result<Option<IsaProbe>> {
        Ok(None)
    }
}

/// Blanket forward so `Box<dyn TestDriver>` satisfies `TestDriver` itself.
/// The daemon hands the worker a boxed driver out of the factory, and the
/// fuzzer engine wants its `D: TestDriver` bound to hold on that boxed
/// value. Without this impl every caller would have to deref the box
/// manually or the engine would have to keep dedicated trait-object
/// fields. Cheap forwarding through `(**self)` lets one engine type cover
/// both concrete-driver and daemon-dispatched call paths.
#[async_trait]
impl<T: TestDriver + ?Sized> TestDriver for Box<T> {
    fn target(&self) -> DutKind {
        (**self).target()
    }
    fn required_transports(&self) -> &[TransportKind] {
        (**self).required_transports()
    }
    async fn prepare(&mut self, dut: &mut Dut) -> Result<()> {
        (**self).prepare(dut).await
    }
    async fn compile(
        &mut self,
        input: &Artifact,
        tools: &heimdall_tools::ToolChain,
    ) -> Result<Artifact> {
        (**self).compile(input, tools).await
    }
    async fn load(&mut self, dut: &mut Dut, image: &Artifact) -> Result<()> {
        (**self).load(dut, image).await
    }
    async fn run(&mut self, dut: &mut Dut, stimulus: &Stimulus) -> Result<Observation> {
        (**self).run(dut, stimulus).await
    }
    async fn observe(&mut self, dut: &mut Dut) -> Result<State> {
        (**self).observe(dut).await
    }
    async fn diff(&self, dut_state: &State, golden_state: &State) -> Verdict {
        (**self).diff(dut_state, golden_state).await
    }
    async fn release(&mut self, dut: &mut Dut) -> Result<()> {
        (**self).release(dut).await
    }
    fn coverage(&self) -> Option<&dyn heimdall_golden::CoverageSource> {
        (**self).coverage()
    }
    async fn probe_isa(&mut self) -> Result<Option<IsaProbe>> {
        (**self).probe_isa().await
    }
}
