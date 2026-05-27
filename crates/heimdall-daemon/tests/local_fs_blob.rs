//! Integration tests for LocalFsBlobStore.

use heimdall_daemon::{BlobId, BlobStore, LocalFsBlobStore};
use tempfile::TempDir;

async fn store() -> (LocalFsBlobStore, TempDir) {
    let tmp = TempDir::new().expect("tempdir");
    let store = LocalFsBlobStore::open(tmp.path().to_path_buf())
        .await
        .expect("open");
    (store, tmp)
}

#[tokio::test]
async fn put_returns_deterministic_id() {
    let (store, _tmp) = store().await;
    let id1 = store.put(b"hello").await.unwrap();
    let id2 = store.put(b"hello").await.unwrap();
    assert_eq!(id1, id2);
    assert_eq!(id1, BlobId::from_bytes(b"hello"));
}

#[tokio::test]
async fn get_returns_identical_bytes() {
    let (store, _tmp) = store().await;
    let id = store.put(b"abcdef").await.unwrap();
    let bytes = store.get(&id).await.unwrap().expect("present");
    assert_eq!(bytes.as_ref(), b"abcdef");
}

#[tokio::test]
async fn exists_reflects_writes() {
    let (store, _tmp) = store().await;
    let id = BlobId::from_bytes(b"unwritten");
    assert!(!store.exists(&id).await.unwrap());
    let written = store.put(b"unwritten").await.unwrap();
    assert!(store.exists(&written).await.unwrap());
}

#[tokio::test]
async fn get_missing_returns_none() {
    let (store, _tmp) = store().await;
    let id = BlobId("0000000000000000000000000000000000000000000000000000000000000000".into());
    assert!(store.get(&id).await.unwrap().is_none());
}

#[tokio::test]
async fn put_is_atomic_no_tmp_left_behind() {
    let (store, tmp) = store().await;
    let _ = store.put(b"atomic").await.unwrap();
    // Walk the root and ensure no .tmp file remains.
    let mut leftover = Vec::new();
    let mut stack = vec![tmp.path().to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut rd = tokio::fs::read_dir(&dir).await.unwrap();
        while let Some(entry) = rd.next_entry().await.unwrap() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|s| s.to_str()) == Some("tmp") {
                leftover.push(path);
            }
        }
    }
    assert!(leftover.is_empty(), "tmp files left behind: {leftover:?}");
}
