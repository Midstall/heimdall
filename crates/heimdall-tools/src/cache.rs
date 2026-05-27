use heimdall_core::Artifact;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;

use crate::trait_def::ToolOpts;

#[derive(Debug, Default)]
pub struct OutputCache {
    entries: Mutex<HashMap<String, Artifact>>,
}

impl OutputCache {
    pub fn key(tool_fp: &str, input_sha: &str, opts: &ToolOpts) -> String {
        let mut h = Sha256::new();
        h.update(tool_fp.as_bytes());
        h.update(b"|");
        h.update(input_sha.as_bytes());
        h.update(b"|");
        h.update(opts.canonical().as_bytes());
        hex::encode(h.finalize())
    }

    pub fn get(&self, key: &str) -> Option<Artifact> {
        self.entries.lock().unwrap().get(key).cloned()
    }

    pub fn put(&self, key: String, artifact: Artifact) {
        self.entries.lock().unwrap().insert(key, artifact);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use heimdall_core::ArtifactKind;

    #[test]
    fn cache_keys_differ_on_opts() {
        let mut a = ToolOpts::default();
        a.kv.insert("x".into(), "1".into());
        let mut b = ToolOpts::default();
        b.kv.insert("x".into(), "2".into());
        let ka = OutputCache::key("tool@v1", "sha", &a);
        let kb = OutputCache::key("tool@v1", "sha", &b);
        assert_ne!(ka, kb);
    }

    #[test]
    fn cache_put_get() {
        let c = OutputCache::default();
        let a = Artifact::new(ArtifactKind::RawBytes, &b"x"[..]);
        c.put("k".into(), a.clone());
        let back = c.get("k").unwrap();
        assert_eq!(back.bytes, a.bytes);
    }
}
