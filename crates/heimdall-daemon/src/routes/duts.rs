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

use crate::dut_registry::{ConnectionStatus, DutRecord};
use crate::server::AppState;
use crate::types::Lease;

#[derive(Serialize)]
struct DutsResponse {
    duts: Vec<DutWithStatus>,
    leases: Vec<Lease>,
}

/// A `DutRecord` augmented with a live `connection_status` probe result.
/// Serialized at request time so callers (web UI, TUI) see fresh state
/// without needing a separate endpoint.
#[derive(Serialize)]
struct DutWithStatus {
    #[serde(flatten)]
    record: DutRecord,
    connection_status: ConnectionStatus,
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
    let duts = records
        .into_iter()
        .zip(statuses)
        .map(|(record, connection_status)| DutWithStatus {
            record,
            connection_status,
        })
        .collect();
    let leases = app.leases.list().await;
    Json(DutsResponse { duts, leases })
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
