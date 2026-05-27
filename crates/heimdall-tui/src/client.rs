//! HTTP + WS client for talking to a heimdall daemon.

use futures::StreamExt;
use heimdall_daemon::{Campaign, Job};
use serde_json::Value;
use std::collections::HashMap;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::protocol::Message;
use tracing::{debug, warn};

use crate::app::{ConnectionStatus, DutRow};
use crate::error::{Result, TuiError};

#[derive(Clone)]
pub struct DaemonClient {
    base_url: String,
    http: reqwest::Client,
}

impl DaemonClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            http: reqwest::Client::new(),
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub async fn list_jobs(&self) -> Result<Vec<Job>> {
        let url = format!("{}/jobs", self.base_url);
        let body: serde_json::Value = self
            .http
            .get(&url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let jobs = body
            .get("jobs")
            .cloned()
            .unwrap_or(serde_json::Value::Array(vec![]));
        Ok(serde_json::from_value(jobs)?)
    }

    pub async fn get_job(&self, id: &str) -> Result<Option<Job>> {
        let url = format!("{}/jobs/{id}", self.base_url);
        let resp = self.http.get(&url).send().await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let resp = resp.error_for_status()?;
        Ok(Some(resp.json().await?))
    }

    /// Fetch the list of configured DUTs plus active leases from `/duts`, and
    /// project them into the flat `DutRow` shape the TUI renders. Cross-
    /// references each DUT's id against the lease list to fill `leased_by`.
    pub async fn list_duts(&self) -> Result<Vec<DutRow>> {
        let url = format!("{}/duts", self.base_url);
        let body: serde_json::Value = self
            .http
            .get(&url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        // Build dut-id -> holder-job-id from the leases array.
        let mut lease_by_dut: HashMap<String, String> = HashMap::new();
        if let Some(leases) = body.get("leases").and_then(|v| v.as_array()) {
            for lease in leases {
                let dut = lease.get("dut").and_then(|v| v.as_str()).map(str::to_owned);
                let holder = lease
                    .get("holder")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned);
                if let (Some(d), Some(h)) = (dut, holder) {
                    lease_by_dut.insert(d, h);
                }
            }
        }

        let duts_v = body
            .get("duts")
            .cloned()
            .unwrap_or(serde_json::Value::Array(vec![]));
        let raw_duts: Vec<serde_json::Value> = serde_json::from_value(duts_v)?;

        let mut rows = Vec::with_capacity(raw_duts.len());
        for entry in raw_duts {
            let id = entry
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
            let kind = entry
                .get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
            let chip_serial = entry
                .get("chip_serial")
                .and_then(|v| v.as_str())
                .map(str::to_owned);
            let jtag_driver = entry
                .get("jtag")
                .and_then(|j| j.get("driver"))
                .and_then(|v| v.as_str())
                .map(str::to_owned);
            let connection_status = entry
                .get("connection_status")
                .and_then(|v| v.as_str())
                .map(|s| match s {
                    "connected" => ConnectionStatus::Connected,
                    "disconnected" => ConnectionStatus::Disconnected,
                    _ => ConnectionStatus::Unknown,
                })
                .unwrap_or_default();
            let leased_by = lease_by_dut.get(&id).cloned();
            rows.push(DutRow {
                id,
                kind,
                chip_serial,
                jtag_driver,
                leased_by,
                connection_status,
            });
        }
        Ok(rows)
    }

    pub async fn list_campaigns(&self) -> Result<Vec<Campaign>> {
        let url = format!("{}/campaigns", self.base_url);
        let body: serde_json::Value = self
            .http
            .get(&url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let campaigns = body
            .get("campaigns")
            .cloned()
            .unwrap_or(serde_json::Value::Array(vec![]));
        Ok(serde_json::from_value(campaigns)?)
    }

    /// Spawn a background task that subscribes to the daemon's /events WS and
    /// forwards each Event JSON value down a channel. Returns the receiver.
    /// When the connection drops the task exits and the channel closes.
    pub async fn subscribe_events(&self) -> Result<mpsc::UnboundedReceiver<Value>> {
        let ws_url = ws_url_from_http(&self.base_url)?;
        let (tx, rx) = mpsc::unbounded_channel();
        let url = format!("{ws_url}/events");
        debug!(%url, "ws connecting");

        let (ws_stream, _) = tokio_tungstenite::connect_async(&url).await?;
        let (_write, mut read) = ws_stream.split();

        tokio::spawn(async move {
            while let Some(msg) = read.next().await {
                match msg {
                    Ok(Message::Text(t)) => match serde_json::from_str::<Value>(&t) {
                        Ok(v) => {
                            if tx.send(v).is_err() {
                                return;
                            }
                        }
                        Err(e) => warn!(error = %e, "parsing ws frame"),
                    },
                    Ok(Message::Close(_)) | Err(_) => return,
                    _ => continue,
                }
            }
        });

        Ok(rx)
    }
}

// TuiError carries a `reqwest::Error` variant whose internal state pushes
// the enum past clippy's `result_large_err` threshold. The lint is correct
// in principle, but boxing every result for a tiny TUI app is more noise
// than the cost it avoids.
#[allow(clippy::result_large_err)]
fn ws_url_from_http(http_url: &str) -> Result<String> {
    if let Some(rest) = http_url.strip_prefix("http://") {
        Ok(format!("ws://{rest}"))
    } else if let Some(rest) = http_url.strip_prefix("https://") {
        Ok(format!("wss://{rest}"))
    } else {
        Err(TuiError::BadUrl(http_url.into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ws_url_translation() {
        assert_eq!(
            ws_url_from_http("http://localhost:7777").unwrap(),
            "ws://localhost:7777"
        );
        assert_eq!(
            ws_url_from_http("https://rig.lab:7777").unwrap(),
            "wss://rig.lab:7777"
        );
        assert!(ws_url_from_http("ftp://x").is_err());
    }
}
