//! Live spike smoke test. Skipped automatically when no spike binary is
//! reachable; flips on when `HEIMDALL_SPIKE_BIN` points at a working spike.
//! Guards the `SpikeOneShot` path against `--max-cycles`-style regressions
//! by exercising the `-d` + `--debug-cmd` loop against a real binary.

#![cfg(feature = "spike")]

use std::path::PathBuf;

use heimdall_core::{Artifact, ArtifactKind, DutKind, StepBudget};
use heimdall_golden::{GoldenModel, spike::SpikeOneShot};

fn spike_binary() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("HEIMDALL_SPIKE_BIN") {
        let path = PathBuf::from(p);
        if path.is_file() {
            return Some(path);
        }
    }
    // Best-effort PATH lookup; doesn't fail the test if absent.
    if let Ok(out) = std::process::Command::new("which").arg("spike").output() {
        if out.status.success() {
            let p = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// Locate the bring-up firmware (`li a0, 0x42; ebreak`) that's compiled
/// into the workspace under `testdata/river-bringup/hello.elf`. Spike
/// requires section headers and other ELF bits beyond what our minimal
/// hand-built ELF carries, so we use the real artifact.
fn bringup_elf_path() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // crates/
    p.pop(); // workspace root
    p.push("testdata/river-bringup/hello.elf");
    p
}

/// Wrap raw RV64 machine code in the same minimal ELF the fuzz path uses
/// (one PT_LOAD at 0x10000 + one null SHDR) and confirm spike's fesvr
/// loader doesn't trip the `e_shstrndx < e_shnum` assertion. Guards the
/// raw_rv64_elf wrapper against regressions that would silently break
/// every fuzz iteration's golden side.
#[tokio::test]
async fn spike_loads_raw_rv64_wrapper_without_fesvr_assertion() {
    let Some(spike) = spike_binary() else {
        eprintln!("spike binary not found; skipping");
        return;
    };
    if std::process::Command::new("dtc")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("dtc not on PATH; skipping");
        return;
    }
    // The same byte-level builder the fuzzer's RawBytesToElfRiscv tool
    // uses. Replicated here so this crate doesn't need a circular dep on
    // heimdall-tools just to test ELF compat.
    let elf_bytes = wrap_rv64_le_for_test(
        &[0x13, 0x05, 0x20, 0x04, 0x73, 0x00, 0x10, 0x00], // li a0,0x42; ebreak
        0x10000,
    );
    let mut g = SpikeOneShot::new(spike, DutKind::RiverRc1Nano);
    g.load(&Artifact::new(ArtifactKind::ElfRiscv, elf_bytes))
        .await
        .expect("load");
    g.step(StepBudget::cycles(16))
        .await
        .expect("step must not trip spike's fesvr SHDR assertion");
    let state = g.observe().await.expect("observe");
    match state.fields.get("x10") {
        Some(heimdall_core::ValueRepr::U64(v)) => assert_eq!(*v, 0x42),
        other => panic!("x10 missing or wrong variant: {other:?}"),
    }
}

/// Mirror of `heimdall_tools::raw_rv64_elf::wrap_rv64_le`. Kept inline so
/// this crate doesn't pull heimdall-tools as a dev-dep just for one
/// helper. If the wrapper ever changes shape, this needs to follow.
fn wrap_rv64_le_for_test(code: &[u8], load_addr: u64) -> Vec<u8> {
    const EHDR: usize = 0x40;
    const PHDR: usize = 0x38;
    const SHDR: usize = 0x40;
    let payload_off = EHDR + PHDR;
    let shstrtab_off = payload_off + code.len();
    let shstrtab_size: usize = 1;
    let shdr_off = shstrtab_off + shstrtab_size;
    let total = shdr_off + 2 * SHDR;
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
    buf[60..62].copy_from_slice(&2u16.to_le_bytes());
    buf[62..64].copy_from_slice(&1u16.to_le_bytes()); // e_shstrndx
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
    // shdr[1] = .shstrtab.
    let sh1 = shdr_off + SHDR;
    buf[sh1 + 4..sh1 + 8].copy_from_slice(&3u32.to_le_bytes()); // SHT_STRTAB
    buf[sh1 + 24..sh1 + 32].copy_from_slice(&(shstrtab_off as u64).to_le_bytes());
    buf[sh1 + 32..sh1 + 40].copy_from_slice(&(shstrtab_size as u64).to_le_bytes());
    buf
}

#[tokio::test]
async fn spike_runs_a_bounded_program_via_debug_cmd() {
    let Some(spike) = spike_binary() else {
        eprintln!("spike binary not found; set HEIMDALL_SPIKE_BIN to run this test");
        return;
    };
    let elf_path = bringup_elf_path();
    let Ok(elf_bytes) = std::fs::read(&elf_path) else {
        eprintln!(
            "bring-up firmware missing at {}; skipping",
            elf_path.display()
        );
        return;
    };

    // Spike needs the `dtc` binary on PATH to generate the bootrom that
    // jumps from its reset vector at 0x1000 into the ELF's entry. The
    // daemon is expected to be deployed in an env that has dtc; the test
    // skips if it isn't reachable so a thin sandbox doesn't surface as a
    // false negative.
    if std::process::Command::new("dtc")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("dtc not on PATH; skipping spike smoke (spike's bootrom needs dtc)");
        return;
    }
    let mut g = SpikeOneShot::new(spike, DutKind::RiverRc1Nano);
    g.load(&Artifact::new(ArtifactKind::ElfRiscv, elf_bytes))
        .await
        .expect("load");
    // Bounded execution: 16 instructions is plenty for li+ebreak+trap fallout.
    g.step(StepBudget::cycles(16)).await.expect("step");
    let state = g.observe().await.expect("observe");

    // `li a0, 0x42` must show in the commit log as a write to x10.
    let a0 = state
        .fields
        .get("x10")
        .expect("x10 must appear in the spike observe state");
    match a0 {
        heimdall_core::ValueRepr::U64(v) => assert_eq!(*v, 0x42, "x10 must be 0x42"),
        other => panic!("x10 wrong variant: {other:?}"),
    }
}

/// End-to-end proof that `SpikeOneShot::observe` captures the LAST write
/// per register against a real spike commit log, not the first. Mirrors
/// the structure a fuzz program has post-prologue: an early write that
/// zeroes the register, then a later write that sets the real value.
///
/// Background: when the fuzz `RawAsmGen` prepended a 31-insn
/// register-zeroing prologue, the counterpart River team initially
/// suspected the spike harness was first-write-wins. This test pins the
/// real-binary end-to-end behaviour so any regression of that kind
/// fails here, with the exact program bytes and expected register
/// value visible in the test source.
#[tokio::test]
async fn spike_captures_last_write_per_register_against_real_binary() {
    let Some(spike) = spike_binary() else {
        eprintln!("spike binary not found; set HEIMDALL_SPIKE_BIN to run this test");
        return;
    };
    if std::process::Command::new("dtc")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("dtc not on PATH; skipping (spike's bootrom needs dtc)");
        return;
    }
    // Program: addi x10, x0, 0  ; first write (prologue-style zeroing)
    //          addi x10, x0, 66 ; second write (body-style)
    //          ebreak           ; halt
    let code: [u8; 12] = [
        0x13, 0x05, 0x00, 0x00, // 0x00000513  addi x10, x0, 0
        0x13, 0x05, 0x20, 0x04, // 0x04200513  addi x10, x0, 0x42
        0x73, 0x00, 0x10, 0x00, // 0x00100073  ebreak
    ];
    let elf_bytes = wrap_rv64_le_for_test(&code, 0x10000);
    let mut g = SpikeOneShot::new(spike, DutKind::RiverRc1Nano);
    g.load(&Artifact::new(ArtifactKind::ElfRiscv, elf_bytes))
        .await
        .expect("load");
    g.step(StepBudget::cycles(60)).await.expect("step");
    let state = g.observe().await.expect("observe");
    match state.fields.get("x10") {
        Some(heimdall_core::ValueRepr::U64(v)) => assert_eq!(
            *v, 0x42,
            "x10 must equal the LAST write (0x42), not the first (0)",
        ),
        other => panic!("x10 missing or wrong variant: {other:?}"),
    }
}
