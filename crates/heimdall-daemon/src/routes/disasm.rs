//! `GET /jobs/:id/disasm`. Render the program a job is running as a
//! structured disassembly listing. Today only `BootRiverElf` carries an
//! addressable program directly on the job. The route refuses
//! disassembly for kinds that don't (fuzz programs are generated
//! per-iteration inside the engine, not stored on the job). The
//! Aegis-bitstream variants get an explicit "not yet supported" so
//! the UI can render the right hint instead of an opaque 500.
//!
//! Error responses carry an `i18n_key` + `i18n_args` envelope (in
//! addition to the English `error` fallback) so the frontend can
//! localize them in the operator's language, matching the JobLog
//! event pattern.
//!
//! The disassembler is instantiated per request (it's effectively
//! stateless, just a binary path) and respects the
//! `HEIMDALL_LLVM_OBJDUMP_BIN` env override so a daemon running in a
//! sealed environment can pin a specific nix-store path without
//! relying on PATH lookups.

use std::collections::BTreeMap;

use axum::{
    Json, Router,
    body::Body,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use base64::Engine as _;
use heimdall_core::{Artifact, ArtifactKind};
use heimdall_disasm::{DisasmListing, DisasmOpts, Disassembler, LlvmObjdumpDisassembler};
use serde::Serialize;
use thiserror::Error;

use crate::server::AppState;
use crate::types::{JobId, JobKind};

pub fn router() -> Router<AppState> {
    Router::new().route("/jobs/:id/disasm", get(disasm))
}

/// Response shape for `/jobs/:id/disasm`. Carries the listing plus an
/// optional `iter` for Fuzz jobs (the engine sets this through the
/// program observer so the UI can show "showing iter N of M"). For
/// jobs with a single static program (e.g. BootRiverElf) `iter` is
/// `None`.
#[derive(Serialize)]
struct DisasmResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    iter: Option<u64>,
    #[serde(flatten)]
    listing: DisasmListing,
}

/// Concrete error surface for the disasm route. Each variant maps
/// to a specific (status, i18n_key, args) triple, which
/// `IntoResponse` serialises into the envelope the frontend
/// localises.
#[derive(Debug, Error)]
enum DisasmRouteError {
    #[error("job not found")]
    NotFound,
    #[error("internal error: {0}")]
    Internal(String),
    #[error(
        "aegis bitstream rendering is not yet supported; \
         a netlist->Verilog renderer is the planned path"
    )]
    AegisUnsupported,
    #[error(
        "fuzz job has not loaded any iter yet; check back \
         once the first iter generates a program"
    )]
    NoIterYet,
    #[error("job kind `{kind}` has no executable program to disassemble")]
    NoProgram { kind: &'static str },
    #[error("elf_b64 decode: {0}")]
    ElfDecode(String),
    #[error("disasm: {0}")]
    Disassembler(String),
}

impl From<crate::error::DaemonError> for DisasmRouteError {
    fn from(e: crate::error::DaemonError) -> Self {
        Self::Internal(e.to_string())
    }
}

impl DisasmRouteError {
    fn status(&self) -> StatusCode {
        match self {
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Internal(_) | Self::Disassembler(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::AegisUnsupported
            | Self::NoIterYet
            | Self::NoProgram { .. }
            | Self::ElfDecode(_) => StatusCode::BAD_REQUEST,
        }
    }

    /// i18n catalog key for this error. `None` for errors the
    /// daemon doesn't expect operators to want translated (`Internal`
    /// surfaces a backend bug. The English text is most useful).
    fn i18n_key(&self) -> Option<&'static str> {
        match self {
            Self::AegisUnsupported => Some("web.disasm.error.aegis_unsupported"),
            Self::NoIterYet => Some("web.disasm.error.no_iter_yet"),
            Self::NoProgram { .. } => Some("web.disasm.error.no_program"),
            Self::ElfDecode(_) => Some("web.disasm.error.elf_decode"),
            _ => None,
        }
    }

    fn i18n_args(&self) -> BTreeMap<String, String> {
        let mut args = BTreeMap::new();
        match self {
            Self::NoProgram { kind } => {
                args.insert("kind".into(), (*kind).to_string());
            }
            Self::ElfDecode(detail) => {
                args.insert("detail".into(), detail.clone());
            }
            _ => {}
        }
        args
    }
}

#[derive(Serialize)]
struct DisasmErrorBody {
    /// Server-rendered fallback. Clients that don't know the
    /// `i18n_key` can render this verbatim.
    error: String,
    /// Stable i18n catalog key for the error. Absent for variants
    /// the daemon doesn't expect operators to want translated.
    #[serde(skip_serializing_if = "Option::is_none")]
    i18n_key: Option<&'static str>,
    /// Named-argument substitutions for `i18n_key`. Empty for
    /// variants whose translation has no placeholders.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    i18n_args: BTreeMap<String, String>,
}

impl IntoResponse for DisasmRouteError {
    fn into_response(self) -> Response {
        let body = DisasmErrorBody {
            error: self.to_string(),
            i18n_key: self.i18n_key(),
            i18n_args: self.i18n_args(),
        };
        let json = serde_json::to_vec(&body)
            .unwrap_or_else(|_| b"{\"error\":\"serialize failed\"}".to_vec());
        Response::builder()
            .status(self.status())
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .body(Body::from(json))
            .unwrap()
    }
}

async fn disasm(
    State(app): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<DisasmResponse>, DisasmRouteError> {
    let job_id = JobId(id);
    let job = app
        .store
        .get_job(job_id)
        .await?
        .ok_or(DisasmRouteError::NotFound)?;

    let (bytes, kind, iter) = match &job.kind {
        JobKind::BootRiverElf { elf_b64, .. } => {
            let elf = base64::engine::general_purpose::STANDARD
                .decode(elf_b64)
                .map_err(|e| DisasmRouteError::ElfDecode(e.to_string()))?;
            (elf, ArtifactKind::ElfRiscv, None)
        }
        // Aegis bitstreams need a netlist+Verilog renderer, not a CPU
        // disassembler. Surface that distinction explicitly so the UI
        // can show a precise hint and the operator doesn't waste time
        // looking for an llvm-objdump misconfiguration.
        JobKind::LoadAegisBitstream { .. } | JobKind::RunAegisVector { .. } => {
            return Err(DisasmRouteError::AegisUnsupported);
        }
        // Fuzz programs are generated per-iter inside the engine
        // and pushed into the loaded-program cache as they go. The
        // route serves whichever iter is most recent so the UI
        // tracks the running fuzz session live. Once the job is
        // terminal the entry sticks around for post-mortem
        // inspection. When the in-memory cache is cold (e.g.
        // immediately after a daemon restart for a historical
        // job), fall back to the durable JobStore/BlobStore
        // mapping the worker writes at end-of-run.
        JobKind::Fuzz { .. } => match app.loaded_programs.latest(job_id) {
            Some(program) => (program.bytes, program.kind, Some(program.iter)),
            None => {
                let Some(reference) = app.store.get_job_program(job_id).await? else {
                    return Err(DisasmRouteError::NoIterYet);
                };
                let Some(bytes) = app.blobs.get(&reference.blob_id).await? else {
                    return Err(DisasmRouteError::NoIterYet);
                };
                (bytes.to_vec(), reference.kind, reference.iter)
            }
        },
        JobKind::MockHello | JobKind::Named { .. } => {
            return Err(DisasmRouteError::NoProgram {
                kind: job.kind.tag(),
            });
        }
    };

    let disassembler = build_disassembler();
    let opts = DisasmOpts {
        extensions: enabled_extensions_for_dut(&app, &job.dut),
        ..DisasmOpts::default()
    };
    let listing = disassembler
        .disassemble(&Artifact::new(kind, bytes), &opts)
        .await
        .map_err(|e| DisasmRouteError::Disassembler(e.to_string()))?;
    Ok(Json(DisasmResponse { iter, listing }))
}

/// Resolve "what extensions does this DUT actually have enabled"
/// for the disassembler's `--mattr` flag. Precedence:
///
/// 1. Live JTAG probe (most authoritative: it read `misa` off the
///    silicon)
/// 2. heimdall.toml `[dut.isa]` extensions list
/// 3. Empty (base ISA only)
///
/// We deliberately do NOT widen this to a "supported" superset:
/// the strictness is the feature. If codegen accidentally emits a
/// `mul` for a DUT without M, the listing will show `<unknown>`
/// and the bug surfaces in review instead of silently looking
/// legitimate.
fn enabled_extensions_for_dut(app: &AppState, dut: &heimdall_core::DutId) -> Vec<String> {
    if let Some(probed) = app.isa_probes.get(dut) {
        return probed.extensions.clone();
    }
    if let Some(rec) = app.dut_registry.lookup(dut)
        && let Some(spec) = rec.isa.as_ref()
    {
        return spec.extensions.clone();
    }
    Vec::new()
}

/// Default-build the disassembler. Honours `HEIMDALL_LLVM_OBJDUMP_BIN`
/// (mirrors the spike-smoke env override) so deployments can pin a
/// specific build.
fn build_disassembler() -> LlvmObjdumpDisassembler {
    match std::env::var("HEIMDALL_LLVM_OBJDUMP_BIN") {
        Ok(p) if !p.is_empty() => LlvmObjdumpDisassembler::new(p),
        _ => LlvmObjdumpDisassembler::from_path(),
    }
}
