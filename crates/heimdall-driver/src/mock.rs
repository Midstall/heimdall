use async_trait::async_trait;
use heimdall_core::{
    Artifact, ArtifactKind, DutKind, Evidence, FailureKind, Observation, State, Stimulus, Verdict,
};
use heimdall_transport::TransportKind;
use std::time::Duration;

use crate::error::DriverError;
use crate::trait_def::{Dut, Result, TestDriver};

pub struct MockDriverSiliconCoverage {
    bits: Vec<u8>,
}

impl MockDriverSiliconCoverage {
    pub fn buckets() -> usize {
        1024
    }

    pub fn from_state(state: &heimdall_core::State) -> Self {
        use std::hash::{Hash, Hasher};
        let mut bits = vec![0u8; Self::buckets()];
        for key in state.fields.keys() {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            // Use a different salt than MockCoverage so the two bitmaps
            // produce different bits for the same key (otherwise the
            // sim and silicon "coverage" trivially equal each other).
            "silicon-mock".hash(&mut h);
            key.hash(&mut h);
            let v = h.finish();
            let idx = (v as usize) & (Self::buckets() * 8 - 1);
            let byte = idx / 8;
            let bit = (idx % 8) as u8;
            bits[byte] |= 1 << bit;
        }
        Self { bits }
    }
}

impl heimdall_golden::CoverageSource for MockDriverSiliconCoverage {
    fn snapshot(&self) -> Vec<u8> {
        self.bits.clone()
    }
}

pub struct MockDriver {
    target: DutKind,
    pub prepared: bool,
    pub loaded_sha: Option<String>,
    pub fixed_state: State,
    last_silicon_coverage: Option<MockDriverSiliconCoverage>,
}

impl MockDriver {
    pub fn new(target: DutKind) -> Self {
        Self {
            target,
            prepared: false,
            loaded_sha: None,
            fixed_state: State::new(),
            last_silicon_coverage: None,
        }
    }

    pub fn with_state(mut self, state: State) -> Self {
        self.fixed_state = state;
        self
    }
}

#[async_trait]
impl TestDriver for MockDriver {
    fn target(&self) -> DutKind {
        self.target
    }
    fn required_transports(&self) -> &[TransportKind] {
        &[]
    }

    async fn prepare(&mut self, _dut: &mut Dut) -> Result<()> {
        self.prepared = true;
        Ok(())
    }

    async fn compile(
        &mut self,
        input: &Artifact,
        _tools: &heimdall_tools::ToolChain,
    ) -> Result<Artifact> {
        // Identity transformation for tests.
        let mut a = input.clone();
        if !matches!(
            a.kind,
            ArtifactKind::ElfRiscv | ArtifactKind::Bitstream { .. }
        ) {
            a.kind = ArtifactKind::ElfRiscv;
        }
        Ok(a)
    }

    async fn load(&mut self, _dut: &mut Dut, image: &Artifact) -> Result<()> {
        if !self.prepared {
            return Err(DriverError::State("load before prepare"));
        }
        self.loaded_sha = Some(image.sha256());
        Ok(())
    }

    async fn run(&mut self, _dut: &mut Dut, _stim: &Stimulus) -> Result<Observation> {
        if self.loaded_sha.is_none() {
            return Err(DriverError::State("run before load"));
        }
        Ok(Observation::new(
            self.fixed_state.clone(),
            Duration::from_millis(1),
        ))
    }

    async fn observe(&mut self, _dut: &mut Dut) -> Result<State> {
        let state = self.fixed_state.clone();
        self.last_silicon_coverage = Some(MockDriverSiliconCoverage::from_state(&state));
        Ok(state)
    }

    fn coverage(&self) -> Option<&dyn heimdall_golden::CoverageSource> {
        self.last_silicon_coverage
            .as_ref()
            .map(|c| c as &dyn heimdall_golden::CoverageSource)
    }

    async fn diff(&self, dut_state: &State, golden_state: &State) -> Verdict {
        for (k, v) in &golden_state.fields {
            if let Some(got) = dut_state.fields.get(k) {
                if got != v {
                    return Verdict::Fail {
                        kind: FailureKind::DiffMismatch {
                            field: k.clone(),
                            got: got.clone(),
                            expected: v.clone(),
                        },
                        evidence: vec![Evidence {
                            label: "field".into(),
                            detail: k.clone(),
                        }],
                    };
                }
            }
        }
        Verdict::Pass
    }

    async fn release(&mut self, _dut: &mut Dut) -> Result<()> {
        self.prepared = false;
        self.loaded_sha = None;
        Ok(())
    }
}
