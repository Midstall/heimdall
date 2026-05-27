use async_trait::async_trait;
use heimdall_core::{Artifact, ArtifactKind, DutKind};
use std::collections::BTreeMap;

use crate::error::ToolError;

pub type Result<T> = std::result::Result<T, ToolError>;

#[derive(Debug, Clone)]
pub struct TargetSpec {
    pub dut_kind: DutKind,
    pub desired_output: ArtifactKind,
}

#[derive(Debug, Clone, Default)]
pub struct ToolOpts {
    pub kv: BTreeMap<String, String>,
}

impl ToolOpts {
    pub fn canonical(&self) -> String {
        let mut s = String::new();
        for (k, v) in &self.kv {
            s.push_str(k);
            s.push('=');
            s.push_str(v);
            s.push(';');
        }
        s
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn supports(&self, input: &Artifact, target: &TargetSpec) -> bool;
    async fn run(&self, input: &Artifact, target: &TargetSpec, opts: &ToolOpts)
    -> Result<Artifact>;
    fn version_fingerprint(&self) -> String;
}
