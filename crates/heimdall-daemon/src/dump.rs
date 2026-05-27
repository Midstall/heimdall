//! Daemon snapshot dump/restore.
//!
//! Serializes the full `JobStore` + `BlobStore` state as a tar archive:
//!
//! ```text
//! manifest.json        Snapshot manifest (version, counts, timestamp)
//! jobs.jsonl           One serialized Job per line
//! campaigns.jsonl      One serialized Campaign per line
//! events.jsonl         One [id, event] tuple per line
//! blobs/<sha256-hex>   Raw blob bytes
//! ```
//!
//! Restore preserves all IDs (jobs, campaigns, events) so cross-references
//! survive the round trip. Requires the destination store to implement the
//! `import_*` methods on `JobStore`.

use std::io::{Read, Write};

use serde::{Deserialize, Serialize};

use crate::error::{DaemonError, Result};
use crate::store::{BlobStore, JobStore};
use crate::types::{EventId, JobFilter};

/// Current dump format version. Bump when the layout changes incompatibly.
pub const DUMP_FORMAT_VERSION: u32 = 1;

/// Snapshot manifest written as `manifest.json` inside the tar.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub format_version: u32,
    pub heimdall_version: String,
    pub exported_at: chrono::DateTime<chrono::Utc>,
    pub job_count: u64,
    pub campaign_count: u64,
    pub event_count: u64,
    pub blob_count: u64,
}

/// Summary returned from `restore`.
#[derive(Debug, Clone, Default)]
pub struct RestoreStats {
    pub jobs_restored: u64,
    pub campaigns_restored: u64,
    pub events_restored: u64,
    pub blobs_restored: u64,
}

/// Stream the full daemon state to `writer` as a tar archive.
///
/// The writer is moved (and consumed) so tests can use `Vec<u8>` cleanly.
pub async fn dump<W>(store: &dyn JobStore, blobs: &dyn BlobStore, writer: W) -> Result<Manifest>
where
    W: Write + Send + 'static,
{
    // Collect snapshot data BEFORE building the tar so we can populate the
    // manifest counts. JobStore/BlobStore traits are async but tar is sync,
    // so we read first then write the tar in one pass on a blocking thread.
    let jobs = store.list_jobs(JobFilter::default()).await?;
    let campaigns = store.list_campaigns(None).await?;
    let events = store.list_events_since(EventId(0), u32::MAX).await?;
    let blob_ids = blobs.list_ids().await?;
    let mut blob_bytes: Vec<(String, Vec<u8>)> = Vec::with_capacity(blob_ids.len());
    for id in &blob_ids {
        let bytes = blobs
            .get(id)
            .await?
            .ok_or_else(|| DaemonError::DumpFormat(format!("blob {id:?} listed but missing")))?;
        blob_bytes.push((id.0.clone(), bytes.to_vec()));
    }

    let manifest = Manifest {
        format_version: DUMP_FORMAT_VERSION,
        heimdall_version: heimdall_core::VERSION.to_string(),
        exported_at: chrono::Utc::now(),
        job_count: jobs.len() as u64,
        campaign_count: campaigns.len() as u64,
        event_count: events.len() as u64,
        blob_count: blob_bytes.len() as u64,
    };

    let manifest_for_tar = manifest.clone();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let mut tar = tar::Builder::new(writer);

        write_entry(
            &mut tar,
            "manifest.json",
            serde_json::to_vec_pretty(&manifest_for_tar)?.as_slice(),
        )?;

        let mut jobs_jsonl = Vec::new();
        for job in &jobs {
            jobs_jsonl.extend_from_slice(serde_json::to_string(job)?.as_bytes());
            jobs_jsonl.push(b'\n');
        }
        write_entry(&mut tar, "jobs.jsonl", &jobs_jsonl)?;

        let mut camp_jsonl = Vec::new();
        for c in &campaigns {
            camp_jsonl.extend_from_slice(serde_json::to_string(c)?.as_bytes());
            camp_jsonl.push(b'\n');
        }
        write_entry(&mut tar, "campaigns.jsonl", &camp_jsonl)?;

        let mut ev_jsonl = Vec::new();
        for (id, ev) in &events {
            let line = serde_json::to_string(&(id, ev))?;
            ev_jsonl.extend_from_slice(line.as_bytes());
            ev_jsonl.push(b'\n');
        }
        write_entry(&mut tar, "events.jsonl", &ev_jsonl)?;

        for (id, bytes) in &blob_bytes {
            write_entry(&mut tar, &format!("blobs/{id}"), bytes)?;
        }

        tar.finish().map_err(DaemonError::Io)?;
        Ok(())
    })
    .await
    .map_err(|e| DaemonError::DumpFormat(format!("dump task panicked: {e}")))??;

    Ok(manifest)
}

/// Read a tar archive produced by `dump` and re-populate the given stores.
pub async fn restore<R>(
    store: &dyn JobStore,
    blobs: &dyn BlobStore,
    reader: R,
) -> Result<RestoreStats>
where
    R: Read + Send + 'static,
{
    // Decode the whole tar on a blocking thread, then apply via the async
    // store traits on the current runtime.
    let entries = tokio::task::spawn_blocking(move || -> Result<Vec<(String, Vec<u8>)>> {
        let mut archive = tar::Archive::new(reader);
        let mut out = Vec::new();
        for entry in archive.entries().map_err(DaemonError::Io)? {
            let mut entry = entry.map_err(DaemonError::Io)?;
            let path = entry
                .path()
                .map_err(DaemonError::Io)?
                .to_string_lossy()
                .into_owned();
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf).map_err(DaemonError::Io)?;
            out.push((path, buf));
        }
        Ok(out)
    })
    .await
    .map_err(|e| DaemonError::DumpFormat(format!("restore task panicked: {e}")))??;

    let mut manifest: Option<Manifest> = None;
    let mut jobs_jsonl: Option<Vec<u8>> = None;
    let mut camp_jsonl: Option<Vec<u8>> = None;
    let mut ev_jsonl: Option<Vec<u8>> = None;
    let mut blob_entries: Vec<(String, Vec<u8>)> = Vec::new();

    for (path, bytes) in entries {
        match path.as_str() {
            "manifest.json" => {
                manifest = Some(serde_json::from_slice(&bytes)?);
            }
            "jobs.jsonl" => jobs_jsonl = Some(bytes),
            "campaigns.jsonl" => camp_jsonl = Some(bytes),
            "events.jsonl" => ev_jsonl = Some(bytes),
            p if p.starts_with("blobs/") => {
                let id = p.trim_start_matches("blobs/").to_string();
                blob_entries.push((id, bytes));
            }
            _ => {
                // Unknown future entries: tolerate so older Heimdall versions
                // can still consume newer dumps minus the unknown bits.
                tracing::warn!(path = %path, "ignoring unknown dump entry");
            }
        }
    }

    let manifest =
        manifest.ok_or_else(|| DaemonError::DumpFormat("dump missing manifest.json".into()))?;
    if manifest.format_version != DUMP_FORMAT_VERSION {
        return Err(DaemonError::DumpFormat(format!(
            "unsupported dump format_version {}, this build expects {DUMP_FORMAT_VERSION}",
            manifest.format_version
        )));
    }

    let mut stats = RestoreStats::default();

    if let Some(b) = camp_jsonl {
        for line in split_lines(&b) {
            let c: crate::types::Campaign = serde_json::from_slice(line)?;
            store.import_campaign(c).await?;
            stats.campaigns_restored += 1;
        }
    }

    if let Some(b) = jobs_jsonl {
        for line in split_lines(&b) {
            let j: crate::types::Job = serde_json::from_slice(line)?;
            store.import_job(j).await?;
            stats.jobs_restored += 1;
        }
    }

    if let Some(b) = ev_jsonl {
        for line in split_lines(&b) {
            let (id, ev): (EventId, crate::types::Event) = serde_json::from_slice(line)?;
            store.import_event(id, ev).await?;
            stats.events_restored += 1;
        }
    }

    for (_id, bytes) in blob_entries {
        // BlobStore::put is content-addressed. The resulting BlobId equals
        // the sha256 of `bytes`, matching the original id by construction.
        let _id_back = blobs.put(&bytes).await?;
        stats.blobs_restored += 1;
    }

    Ok(stats)
}

fn write_entry<W: Write>(tar: &mut tar::Builder<W>, name: &str, bytes: &[u8]) -> Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o644);
    header.set_mtime(chrono::Utc::now().timestamp() as u64);
    header.set_cksum();
    tar.append_data(&mut header, name, bytes)
        .map_err(DaemonError::Io)
}

fn split_lines(bytes: &[u8]) -> Vec<&[u8]> {
    bytes
        .split(|&b| b == b'\n')
        .filter(|l| !l.is_empty())
        .collect()
}
