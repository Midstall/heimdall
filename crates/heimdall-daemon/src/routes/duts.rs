//! GET /duts: lists configured DUTs (from DutRegistry) and active leases.
//! GET /duts/:id/netlist.svg: renders the DUT's SPICE netlist as SVG.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Serialize;

use crate::dut_registry::{ConnectionStatus, DutRecord, IsaSpec};
use crate::dut_state::DutStateLatest;
use crate::server::AppState;
use crate::types::Lease;

#[derive(Serialize)]
struct DutsResponse {
    duts: Vec<DutWithStatus>,
    leases: Vec<Lease>,
}

/// Where the effective ISA came from. Lets the UI render a hint
/// next to the extension list so an operator can tell at a glance
/// whether the daemon is leaning on a `misa` read off the silicon
/// or on what heimdall.toml claims.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum IsaSource {
    Probe,
    Config,
}

#[derive(Serialize)]
struct EffectiveIsa {
    #[serde(flatten)]
    spec: IsaSpec,
    source: IsaSource,
}

/// A `DutRecord` augmented with a live `connection_status` probe result,
/// the last-known per-source register snapshots, and the resolved
/// effective ISA (probe-then-config precedence). Serialized at request
/// time so a freshly-loaded UI sees current state without waiting for the
/// next observe() cycle.
#[derive(Serialize)]
struct DutWithStatus {
    #[serde(flatten)]
    record: DutRecord,
    connection_status: ConnectionStatus,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    snapshots: Vec<DutStateLatest>,
    /// Live ISA the disassembler / fuzzer will use against this DUT.
    /// `None` when neither a probe nor a config spec exists; UI
    /// can render "unknown" rather than implying base-I default.
    #[serde(skip_serializing_if = "Option::is_none")]
    effective_isa: Option<EffectiveIsa>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/duts", get(list))
        .route("/duts/:id/netlist.svg", get(netlist_svg))
}

async fn list(State(app): State<AppState>) -> Json<DutsResponse> {
    let records: Vec<DutRecord> = app.dut_registry.iter().cloned().collect();
    // Probe all DUTs in parallel so a slow openocd TCP timeout doesn't
    // serialize across the whole list.
    let probes = records.iter().map(|d| d.jtag.probe_connection());
    let statuses = futures::future::join_all(probes).await;
    let mut duts = Vec::with_capacity(records.len());
    for (record, connection_status) in records.into_iter().zip(statuses) {
        let snapshots = app.dut_state_cache.snapshot_for(&record.id).await;
        let effective_isa = resolve_effective_isa(&app, &record);
        duts.push(DutWithStatus {
            record,
            connection_status,
            snapshots,
            effective_isa,
        });
    }
    let leases = app.leases.list().await;
    Json(DutsResponse { duts, leases })
}

/// Probe-first ISA resolution that mirrors what the disasm route
/// and worker use for `--mattr` and codegen. Keeps a single source
/// of truth for "what is the DUT actually running": JTAG `misa` if
/// we have a fresh probe, fall back to `[dut.isa]` in heimdall.toml,
/// and `None` when neither is present so the UI doesn't lie.
fn resolve_effective_isa(app: &AppState, record: &DutRecord) -> Option<EffectiveIsa> {
    if let Some(probed) = app.isa_probes.get(&record.id) {
        return Some(EffectiveIsa {
            spec: IsaSpec::from(&probed),
            source: IsaSource::Probe,
        });
    }
    record.isa.as_ref().map(|spec| EffectiveIsa {
        spec: spec.clone(),
        source: IsaSource::Config,
    })
}

/// Render the SPICE netlist for `id` as SVG. 404 if the DUT has no netlist
/// configured. 500 if the file was removed after startup.
async fn netlist_svg(
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, (StatusCode, String)> {
    use heimdall_core::DutId;
    use heimdall_golden::{SpiceDir, SpiceWatch, render_netlist};

    let dut_id = DutId::new(id.clone());
    let dut = app
        .dut_registry
        .lookup(&dut_id)
        .ok_or((StatusCode::NOT_FOUND, format!("unknown dut `{id}`")))?;

    let netlist_path = dut
        .netlist
        .as_ref()
        .ok_or((StatusCode::NOT_FOUND, format!("dut `{id}` has no netlist")))?;

    let src = std::fs::read_to_string(netlist_path).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("reading {}: {e}", netlist_path.display()),
        )
    })?;

    let watches: Vec<SpiceWatch> = dut
        .spice_watches
        .iter()
        .map(|w| SpiceWatch {
            name: w.name.clone(),
            spice_node: w.spice_node.clone(),
            direction: match w.direction {
                crate::dut_registry::PadDirection::In => SpiceDir::In,
                crate::dut_registry::PadDirection::Out => SpiceDir::Out,
            },
        })
        .collect();

    let svg = render_netlist(&src, None, &watches, 700, 500)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("render: {e}")))?;

    Ok((
        [
            (header::CONTENT_TYPE, "image/svg+xml; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        svg,
    )
        .into_response())
}
