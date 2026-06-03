use thiserror::Error;

#[derive(Debug, Error)]
pub enum DisasmError {
    #[error("artifact kind {kind:?} is not supported by disassembler `{disassembler}`")]
    UnsupportedArtifact {
        disassembler: &'static str,
        kind: heimdall_core::ArtifactKind,
    },
    #[error("llvm-objdump binary `{path}` is not on PATH or is not executable")]
    ObjdumpNotFound { path: String },
    #[error("llvm-objdump exited with status {status}: {stderr}")]
    ObjdumpBadExit { status: i32, stderr: String },
    #[error("could not parse llvm-objdump output: {reason}")]
    ParseError { reason: String },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
