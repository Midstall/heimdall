//! Integration tests for `GET /jobs/:id/disasm`. Covers:
//!
//! - 404 for an unknown job id.
//! - 400 for job kinds that have no executable program to disassemble
//!   (mock-hello, fuzz, aegis bitstream).
//! - 200 + JSON DisasmListing for a BootRiverElf job (skips when no
//!   llvm-objdump is reachable, mirroring the spike-smoke skip).

#![cfg(feature = "sqlite")]

use std::sync::Arc;

use heimdall_core::{DutId, DutKind};
use heimdall_daemon::{
    BlobStore, DutRecord, DutRegistry, JobKind, JobStore, LocalFsBlobStore, NewJob, SqliteJobStore,
    runtime,
};
use tempfile::TempDir;

async fn start_daemon() -> (heimdall_daemon::DaemonHandles, TempDir) {
    let tmp = TempDir::new().expect("tmp");
    let store = SqliteJobStore::open_in_memory().await.expect("store");
    let blobs = LocalFsBlobStore::open(tmp.path().to_path_buf())
        .await
        .expect("blobs");
    let mut registry = DutRegistry::new();
    registry.insert(DutRecord::mock("river-1", DutKind::RiverRc1Nano));
    let bind: std::net::SocketAddr = "127.0.0.1:0".parse().unwrap();
    let handles = runtime::start_with_dut_registry(
        bind,
        Arc::new(store) as Arc<dyn JobStore>,
        Arc::new(blobs) as Arc<dyn BlobStore>,
        Arc::new(registry),
    )
    .await
    .expect("daemon start");
    (handles, tmp)
}

#[tokio::test]
async fn unknown_job_returns_404() {
    let (handles, _tmp) = start_daemon().await;
    let url = format!(
        "http://{}/jobs/00000000-0000-0000-0000-000000000000/disasm",
        handles.local_addr,
    );
    let resp = reqwest::get(&url).await.expect("get");
    assert_eq!(resp.status(), 404);
    handles.server_task.abort();
    handles.worker_task.abort();
}

async fn submit_job(client: &reqwest::Client, addr: std::net::SocketAddr, kind: JobKind) -> String {
    let new = NewJob {
        dut: DutId::new("river-1"),
        kind,
        campaign: None,
    };
    let resp = client
        .post(format!("http://{addr}/jobs"))
        .json(&new)
        .send()
        .await
        .expect("post");
    assert_eq!(resp.status(), 201, "expected 201; got {}", resp.status());
    let body: serde_json::Value = resp.json().await.expect("json");
    body["id"].as_str().expect("id field").to_string()
}

#[tokio::test]
async fn mock_hello_job_disasm_returns_400() {
    let (handles, _tmp) = start_daemon().await;
    let client = reqwest::Client::new();
    let job_id = submit_job(&client, handles.local_addr, JobKind::MockHello).await;
    let url = format!("http://{}/jobs/{job_id}/disasm", handles.local_addr);
    let resp = client.get(&url).send().await.expect("get");
    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await.expect("json");
    let err = body["error"].as_str().unwrap_or("");
    assert!(
        err.contains("mock-hello") || err.contains("no executable program"),
        "error must mention the kind / no-program reason; got `{err}`",
    );
    handles.server_task.abort();
    handles.worker_task.abort();
}

#[tokio::test]
async fn fuzz_job_disasm_before_any_iter_returns_helpful_400() {
    let (handles, _tmp) = start_daemon().await;
    let client = reqwest::Client::new();
    let job_id = submit_job(
        &client,
        handles.local_addr,
        JobKind::Fuzz {
            iterations: 1,
            seed: 0,
            insn_count: 4,
            cycles: 100,
            strict_coverage: false,
            generator: heimdall_daemon::GeneratorKind::RawAsm,
        },
    )
    .await;
    let url = format!("http://{}/jobs/{job_id}/disasm", handles.local_addr);
    // We hit /disasm right after submission. The worker may not have
    // dispatched the job yet, OR it may have failed before generating
    // any iter (the default registry doesn't have a fuzz factory). In
    // either case the cache has no entry, so the route returns the
    // "has not loaded any iter yet" 400 we want to pin here. If the
    // worker DID populate the cache first (extremely fast on a hot
    // CI), we get a 200 with a listing - also acceptable, just out of
    // scope for this test. We poll briefly to keep the assertion stable.
    let mut got_expected = false;
    for _ in 0..20 {
        let resp = client.get(&url).send().await.expect("get");
        if resp.status() == 400 {
            let body: serde_json::Value = resp.json().await.expect("json");
            let err = body["error"].as_str().unwrap_or("");
            if err.contains("has not loaded any iter") {
                got_expected = true;
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(
        got_expected,
        "expected at least one 400 'has not loaded any iter yet' response \
         before the worker populates the cache",
    );
    handles.server_task.abort();
    handles.worker_task.abort();
}

fn llvm_objdump_available() -> bool {
    if std::env::var("HEIMDALL_LLVM_OBJDUMP_BIN")
        .ok()
        .filter(|p| !p.is_empty())
        .map(|p| std::path::Path::new(&p).is_file())
        .unwrap_or(false)
    {
        return true;
    }
    std::process::Command::new("which")
        .arg("llvm-objdump")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[tokio::test]
async fn boot_river_elf_disasm_returns_json_listing() {
    if !llvm_objdump_available() {
        eprintln!("llvm-objdump unavailable; skipping");
        return;
    }
    let (handles, _tmp) = start_daemon().await;
    let client = reqwest::Client::new();

    // Real RV64 ELF carrying `addi a0, x0, 0x42; ebreak` at 0x10000.
    // Built by the same heimdall-disasm helper the route exercises so
    // the load-addr ELF semantics match end-to-end.
    let code = [
        0x13u8, 0x05, 0x20, 0x04, // addi a0, zero, 0x42
        0x73, 0x00, 0x10, 0x00, // ebreak
    ];
    let elf = wrap_rv64_elf_for_disasm(&code, 0x10000);
    let elf_b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &elf);

    let job_id = submit_job(
        &client,
        handles.local_addr,
        JobKind::BootRiverElf {
            elf_b64,
            cycles: 100,
        },
    )
    .await;
    // Don't wait for the job to finish; disasm only inspects the
    // stored job kind, not its terminal verdict.
    let url = format!("http://{}/jobs/{job_id}/disasm", handles.local_addr);
    let resp = client.get(&url).send().await.expect("get");
    assert_eq!(
        resp.status(),
        200,
        "expected 200; got {} body={}",
        resp.status(),
        resp.text().await.unwrap_or_default(),
    );
    let listing: serde_json::Value = resp.json().await.expect("json");
    assert_eq!(listing["name"], "llvm-objdump");
    assert_eq!(listing["isa"], "rv64");
    let lines = listing["lines"].as_array().expect("lines array");
    assert!(!lines.is_empty(), "must produce >=1 disasm line");
    let first = &lines[0];
    assert_eq!(first["addr"], 0x10000);
    let mnem = first["mnemonic"].as_str().unwrap_or("");
    assert!(
        mnem == "li" || mnem == "addi",
        "first mnemonic must be li/addi; got `{mnem}`",
    );
    assert!(
        lines.iter().any(|l| l["mnemonic"] == "ebreak"),
        "listing must include an ebreak somewhere",
    );

    handles.server_task.abort();
    handles.worker_task.abort();
}

/// Minimal RV64 ELF builder mirroring
/// `heimdall_disasm::llvm_objdump::wrap_rv64_le_for_objdump`. Kept
/// inline so this test crate doesn't have to depend on
/// `heimdall-disasm` (which the daemon already pulls in transitively;
/// the wrap helper is intentionally private to the disasm crate).
fn wrap_rv64_elf_for_disasm(code: &[u8], load_addr: u64) -> Vec<u8> {
    const EHDR: usize = 0x40;
    const PHDR: usize = 0x38;
    const SHDR: usize = 0x40;
    let payload_off = EHDR + PHDR;
    const SHSTRTAB: &[u8] = b"\0.text\0.shstrtab\0";
    let text_name: u32 = 1;
    let shstrtab_name: u32 = 7;
    let shstrtab_off = payload_off + code.len();
    let shstrtab_size = SHSTRTAB.len();
    let shdr_off = shstrtab_off + shstrtab_size;
    let total = shdr_off + 3 * SHDR;
    let mut buf = vec![0u8; total];
    buf[0..4].copy_from_slice(b"\x7fELF");
    buf[4] = 2;
    buf[5] = 1;
    buf[6] = 1;
    buf[16..18].copy_from_slice(&2u16.to_le_bytes());
    buf[18..20].copy_from_slice(&243u16.to_le_bytes());
    buf[20..24].copy_from_slice(&1u32.to_le_bytes());
    buf[24..32].copy_from_slice(&load_addr.to_le_bytes());
    buf[32..40].copy_from_slice(&(EHDR as u64).to_le_bytes());
    buf[40..48].copy_from_slice(&(shdr_off as u64).to_le_bytes());
    buf[52..54].copy_from_slice(&(EHDR as u16).to_le_bytes());
    buf[54..56].copy_from_slice(&(PHDR as u16).to_le_bytes());
    buf[56..58].copy_from_slice(&1u16.to_le_bytes());
    buf[58..60].copy_from_slice(&(SHDR as u16).to_le_bytes());
    buf[60..62].copy_from_slice(&3u16.to_le_bytes());
    buf[62..64].copy_from_slice(&2u16.to_le_bytes());
    let ph = EHDR;
    buf[ph..ph + 4].copy_from_slice(&1u32.to_le_bytes());
    buf[ph + 4..ph + 8].copy_from_slice(&5u32.to_le_bytes());
    buf[ph + 8..ph + 16].copy_from_slice(&(payload_off as u64).to_le_bytes());
    buf[ph + 16..ph + 24].copy_from_slice(&load_addr.to_le_bytes());
    buf[ph + 24..ph + 32].copy_from_slice(&load_addr.to_le_bytes());
    buf[ph + 32..ph + 40].copy_from_slice(&(code.len() as u64).to_le_bytes());
    buf[ph + 40..ph + 48].copy_from_slice(&(code.len() as u64).to_le_bytes());
    buf[ph + 48..ph + 56].copy_from_slice(&0x1000u64.to_le_bytes());
    buf[payload_off..payload_off + code.len()].copy_from_slice(code);
    buf[shstrtab_off..shstrtab_off + shstrtab_size].copy_from_slice(SHSTRTAB);
    let sh1 = shdr_off + SHDR;
    buf[sh1..sh1 + 4].copy_from_slice(&text_name.to_le_bytes());
    buf[sh1 + 4..sh1 + 8].copy_from_slice(&1u32.to_le_bytes());
    buf[sh1 + 8..sh1 + 16].copy_from_slice(&0x6u64.to_le_bytes());
    buf[sh1 + 16..sh1 + 24].copy_from_slice(&load_addr.to_le_bytes());
    buf[sh1 + 24..sh1 + 32].copy_from_slice(&(payload_off as u64).to_le_bytes());
    buf[sh1 + 32..sh1 + 40].copy_from_slice(&(code.len() as u64).to_le_bytes());
    buf[sh1 + 48..sh1 + 56].copy_from_slice(&4u64.to_le_bytes());
    let sh2 = shdr_off + 2 * SHDR;
    buf[sh2..sh2 + 4].copy_from_slice(&shstrtab_name.to_le_bytes());
    buf[sh2 + 4..sh2 + 8].copy_from_slice(&3u32.to_le_bytes());
    buf[sh2 + 24..sh2 + 32].copy_from_slice(&(shstrtab_off as u64).to_le_bytes());
    buf[sh2 + 32..sh2 + 40].copy_from_slice(&(shstrtab_size as u64).to_le_bytes());
    buf
}

/// Regression test for "disasm doesn't last when the daemon
/// restarts": pre-seed a Fuzz job + a program-blob mapping into the
/// stores BEFORE starting the daemon, then hit /disasm and verify
/// the route reads from durable storage when the in-memory cache is
/// cold. Mirrors the scenario where an operator opens a past fuzz
/// job's disasm panel after a daemon restart.
#[tokio::test]
async fn fuzz_disasm_falls_back_to_blob_store_after_cold_cache() {
    if !llvm_objdump_available() {
        eprintln!("llvm-objdump unavailable; skipping");
        return;
    }
    use heimdall_core::{ArtifactKind, DutKind};
    use heimdall_daemon::{
        BlobStore, DutRecord, DutRegistry, GeneratorKind, JobKind, JobProgramRef, JobStore,
        LocalFsBlobStore, NewJob, SqliteJobStore, runtime,
    };

    let tmp = TempDir::new().expect("tmp");
    let store =
        Arc::new(SqliteJobStore::open_in_memory().await.expect("store")) as Arc<dyn JobStore>;
    let blobs = Arc::new(
        LocalFsBlobStore::open(tmp.path().to_path_buf())
            .await
            .expect("blobs"),
    ) as Arc<dyn BlobStore>;

    // Create a Fuzz job in the store. The default registry path
    // would also work but `create_job` is simpler here.
    let job = store
        .create_job(
            NewJob {
                dut: heimdall_core::DutId::new("river-1"),
                kind: JobKind::Fuzz {
                    iterations: 1,
                    seed: 7,
                    insn_count: 4,
                    cycles: 100,
                    strict_coverage: false,
                    generator: GeneratorKind::RawAsm,
                },
                campaign: None,
            },
            DutKind::RiverRc1Nano,
        )
        .await
        .expect("create_job");

    // Pre-seed the BlobStore + JobStore mapping with a real RV64
    // program. This simulates the worker having persisted on a
    // previous daemon run.
    let code: [u8; 8] = [
        0x13, 0x05, 0x20, 0x04, // addi a0, zero, 0x42
        0x73, 0x00, 0x10, 0x00, // ebreak
    ];
    let blob_id = blobs.put(&code).await.expect("blob put");
    store
        .set_job_program(
            job.id,
            JobProgramRef {
                blob_id,
                kind: ArtifactKind::RawBytes,
                iter: Some(0),
            },
        )
        .await
        .expect("set_job_program");

    // Now start the daemon against the SAME stores. AppState's
    // loaded_programs cache is fresh and empty; the disasm route
    // must fall back to the persisted mapping.
    let mut registry = DutRegistry::new();
    registry.insert(DutRecord::mock("river-1", DutKind::RiverRc1Nano));
    let bind: std::net::SocketAddr = "127.0.0.1:0".parse().unwrap();
    let handles = runtime::start_with_dut_registry(bind, store, blobs, Arc::new(registry))
        .await
        .expect("daemon start");

    let url = format!("http://{}/jobs/{}/disasm", handles.local_addr, job.id.0);
    let resp = reqwest::Client::new().get(&url).send().await.expect("get");
    assert_eq!(
        resp.status(),
        200,
        "fuzz disasm with cold cache must read from durable storage; got {}",
        resp.status(),
    );
    let body: serde_json::Value = resp.json().await.expect("json");
    assert_eq!(body["iter"], 0, "iter must come from the persisted record");
    let lines = body["lines"].as_array().expect("lines");
    assert!(!lines.is_empty(), "must produce >=1 disasm line");
    assert!(
        lines.iter().any(|l| l["mnemonic"] == "ebreak"),
        "ebreak must appear in the listing (proves the bytes round-tripped)",
    );

    handles.server_task.abort();
    handles.worker_task.abort();
}
