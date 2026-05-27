use async_trait::async_trait;
use heimdall_core::{Artifact, ArtifactKind};

use crate::trait_def::{Result, TargetSpec, Tool, ToolOpts};

/// A tool that takes any input and returns a copy of it with a new kind.
/// Lets tests build chains without external processes.
pub struct MockTool {
    accepts: ArtifactKind,
    produces: ArtifactKind,
    name: String,
}

impl MockTool {
    pub fn new(name: impl Into<String>, accepts: ArtifactKind, produces: ArtifactKind) -> Self {
        Self {
            name: name.into(),
            accepts,
            produces,
        }
    }
}

#[async_trait]
impl Tool for MockTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn supports(&self, input: &Artifact, _target: &TargetSpec) -> bool {
        std::mem::discriminant(&input.kind) == std::mem::discriminant(&self.accepts)
    }
    async fn run(
        &self,
        input: &Artifact,
        _target: &TargetSpec,
        _opts: &ToolOpts,
    ) -> Result<Artifact> {
        let mut out = input.clone();
        out.kind = self.produces.clone();
        Ok(out)
    }
    fn version_fingerprint(&self) -> String {
        format!("mock:{}@v1", self.name)
    }
}
