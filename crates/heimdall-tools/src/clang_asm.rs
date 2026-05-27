use async_trait::async_trait;
use heimdall_core::{Artifact, ArtifactKind};
use std::path::PathBuf;
use std::process::Stdio;
use tempfile::NamedTempFile;
use tokio::process::Command;
use tracing::instrument;

use crate::error::ToolError;
use crate::trait_def::{Result, TargetSpec, Tool, ToolOpts};

pub struct ClangAsm {
    binary: PathBuf,
    triple: String,
}

impl ClangAsm {
    pub fn new(binary: impl Into<PathBuf>, triple: impl Into<String>) -> Self {
        Self {
            binary: binary.into(),
            triple: triple.into(),
        }
    }

    /// Default for River RV64: `clang` with `riscv64-unknown-elf` triple.
    pub fn river_rv64() -> Self {
        Self::new("clang", "riscv64-unknown-elf")
    }

    pub fn river_rv32() -> Self {
        Self::new("clang", "riscv32-unknown-elf")
    }
}

#[async_trait]
impl Tool for ClangAsm {
    fn name(&self) -> &str {
        "clang-asm"
    }

    fn supports(&self, input: &Artifact, _target: &TargetSpec) -> bool {
        matches!(input.kind, ArtifactKind::Asm)
    }

    #[instrument(skip(self, input, _opts), fields(tool = "clang-asm", triple = %self.triple))]
    async fn run(
        &self,
        input: &Artifact,
        _target: &TargetSpec,
        _opts: &ToolOpts,
    ) -> Result<Artifact> {
        let mut src = NamedTempFile::new()?;
        use std::io::Write as _;
        src.write_all(&input.bytes)?;
        let src_path = src.into_temp_path();

        let out = tempfile::Builder::new().suffix(".elf").tempfile()?;
        let out_path = out.into_temp_path();

        let status = Command::new(&self.binary)
            .arg("--target")
            .arg(&self.triple)
            .arg("-nostdlib")
            .arg("-static")
            .arg("-x")
            .arg("assembler")
            .arg(&src_path)
            .arg("-o")
            .arg(&out_path)
            .stdin(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .await?;

        if !status.status.success() {
            return Err(ToolError::BadExit {
                tool: "clang-asm".into(),
                status: status.status.code().unwrap_or(-1),
                stderr: String::from_utf8_lossy(&status.stderr).into_owned(),
            });
        }

        let bytes = tokio::fs::read(&out_path).await?;
        if bytes.is_empty() {
            return Err(ToolError::EmptyOutput {
                tool: "clang-asm".into(),
            });
        }

        let mut artifact = Artifact::new(ArtifactKind::ElfRiscv, bytes);
        artifact
            .provenance
            .tool_chain
            .push(self.version_fingerprint());
        artifact.provenance.parent_sha256 = Some(input.sha256());
        Ok(artifact)
    }

    fn version_fingerprint(&self) -> String {
        format!("clang-asm:{}@{}", self.binary.display(), self.triple)
    }
}
