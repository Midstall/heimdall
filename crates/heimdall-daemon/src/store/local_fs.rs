//! Content-addressed local-filesystem BlobStore. Stores blobs at
//! `<root>/<sha[0..2]>/<sha[2..]>` so a directory never holds more than 256
//! immediate child dirs. Atomic writes via tmp file + rename.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use bytes::Bytes;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tracing::debug;

use crate::error::{DaemonError, Result};
use crate::store::BlobStore;
use crate::types::BlobId;

pub struct LocalFsBlobStore {
    root: PathBuf,
}

impl LocalFsBlobStore {
    /// Construct from a root directory. Creates it if missing.
    pub async fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        fs::create_dir_all(&root).await.map_err(DaemonError::Io)?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn path_for(&self, id: &BlobId) -> PathBuf {
        let sha = &id.0;
        let (prefix, rest) = sha.split_at(2);
        self.root.join(prefix).join(rest)
    }
}

#[async_trait]
impl BlobStore for LocalFsBlobStore {
    async fn put(&self, bytes: &[u8]) -> Result<BlobId> {
        let id = BlobId::from_bytes(bytes);
        let final_path = self.path_for(&id);

        // Idempotent: if already present, skip the write.
        if fs::metadata(&final_path).await.is_ok() {
            debug!(blob = %id, "blob already present; skipping write");
            return Ok(id);
        }

        if let Some(parent) = final_path.parent() {
            fs::create_dir_all(parent).await?;
        }

        // Write to a tmp file in the same directory, then atomically rename.
        let tmp_path = final_path.with_extension("tmp");
        {
            let mut tmp = fs::File::create(&tmp_path).await?;
            tmp.write_all(bytes).await?;
            tmp.flush().await?;
            tmp.sync_all().await?;
        }
        fs::rename(&tmp_path, &final_path).await?;
        debug!(blob = %id, size = bytes.len(), "blob written");
        Ok(id)
    }

    async fn get(&self, id: &BlobId) -> Result<Option<Bytes>> {
        let path = self.path_for(id);
        match fs::read(&path).await {
            Ok(v) => Ok(Some(Bytes::from(v))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(DaemonError::Io(e)),
        }
    }

    async fn exists(&self, id: &BlobId) -> Result<bool> {
        match fs::metadata(self.path_for(id)).await {
            Ok(_) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(DaemonError::Io(e)),
        }
    }

    async fn list_ids(&self) -> Result<Vec<BlobId>> {
        // Walk <root>/<2-char-prefix>/<rest>. Skip non-hex dirs and partial
        // tmp files. Only emit ids that look like sha256 hex.
        let mut out = Vec::new();
        let mut top = match fs::read_dir(&self.root).await {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(DaemonError::Io(e)),
        };
        while let Some(prefix_entry) = top.next_entry().await.map_err(DaemonError::Io)? {
            let prefix = prefix_entry.file_name();
            let prefix = match prefix.to_str() {
                Some(s) if s.len() == 2 && s.chars().all(|c| c.is_ascii_hexdigit()) => s.to_owned(),
                _ => continue,
            };
            let mut inner = match fs::read_dir(prefix_entry.path()).await {
                Ok(d) => d,
                Err(_) => continue,
            };
            while let Some(blob_entry) = inner.next_entry().await.map_err(DaemonError::Io)? {
                let name = blob_entry.file_name();
                let name = match name.to_str() {
                    Some(s) if s.len() == 62 && s.chars().all(|c| c.is_ascii_hexdigit()) => {
                        s.to_owned()
                    }
                    _ => continue,
                };
                out.push(BlobId(format!("{prefix}{name}")));
            }
        }
        Ok(out)
    }
}
