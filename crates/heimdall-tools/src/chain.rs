use std::sync::Arc;

use heimdall_core::Artifact;
use tracing::{debug, instrument};

use crate::cache::OutputCache;
use crate::error::ToolError;
use crate::trait_def::{Result, TargetSpec, Tool, ToolOpts};

pub struct ToolChain {
    tools: Vec<Arc<dyn Tool>>,
    cache: OutputCache,
}

impl ToolChain {
    pub fn new() -> Self {
        Self {
            tools: Vec::new(),
            cache: OutputCache::default(),
        }
    }

    pub fn with(mut self, tool: Arc<dyn Tool>) -> Self {
        self.tools.push(tool);
        self
    }

    /// Run the chain until the artifact's kind matches `target.desired_output`,
    /// or until no tool in the chain accepts the current artifact.
    #[instrument(skip(self, input, opts), fields(target = ?target.desired_output))]
    pub async fn build(
        &self,
        input: Artifact,
        target: &TargetSpec,
        opts: &ToolOpts,
    ) -> Result<Artifact> {
        let mut current = input;
        loop {
            if std::mem::discriminant(&current.kind)
                == std::mem::discriminant(&target.desired_output)
            {
                return Ok(current);
            }
            let next = self
                .tools
                .iter()
                .find(|t| t.supports(&current, target))
                .ok_or_else(|| ToolError::NoMatch(current.kind.clone()))?;
            let key = OutputCache::key(&next.version_fingerprint(), &current.sha256(), opts);
            if let Some(hit) = self.cache.get(&key) {
                debug!(tool = next.name(), "cache hit");
                current = hit;
                continue;
            }
            let produced = next.run(&current, target, opts).await?;
            self.cache.put(key, produced.clone());
            current = produced;
        }
    }
}

impl Default for ToolChain {
    fn default() -> Self {
        Self::new()
    }
}
