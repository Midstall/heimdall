use heimdall_core::{Artifact, ArtifactKind, DutKind};
use heimdall_tools::{MockTool, TargetSpec, ToolChain, ToolError, ToolOpts};
use std::sync::Arc;

#[tokio::test]
async fn chain_runs_to_target() {
    let chain = ToolChain::new()
        .with(Arc::new(MockTool::new(
            "asm-to-obj",
            ArtifactKind::Asm,
            ArtifactKind::RawBytes,
        )))
        .with(Arc::new(MockTool::new(
            "obj-to-elf",
            ArtifactKind::RawBytes,
            ArtifactKind::ElfRiscv,
        )));
    let input = Artifact::new(ArtifactKind::Asm, &b"hello"[..]);
    let target = TargetSpec {
        dut_kind: DutKind::RiverRc1Nano,
        desired_output: ArtifactKind::ElfRiscv,
    };
    let out = chain
        .build(input, &target, &ToolOpts::default())
        .await
        .unwrap();
    assert!(matches!(out.kind, ArtifactKind::ElfRiscv));
}

#[tokio::test]
async fn chain_returns_no_match_when_no_tool_accepts() {
    let chain = ToolChain::new();
    let input = Artifact::new(ArtifactKind::Asm, &b"x"[..]);
    let target = TargetSpec {
        dut_kind: DutKind::RiverRc1Nano,
        desired_output: ArtifactKind::ElfRiscv,
    };
    let err = chain
        .build(input, &target, &ToolOpts::default())
        .await
        .unwrap_err();
    assert!(matches!(err, ToolError::NoMatch(_)));
}
