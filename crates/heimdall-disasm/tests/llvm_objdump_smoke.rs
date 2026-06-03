//! Live llvm-objdump smoke test. Skips when no `llvm-objdump` is
//! reachable; flips on when `HEIMDALL_LLVM_OBJDUMP_BIN` points at one,
//! or when `llvm-objdump` resolves via PATH. Mirrors the spike_smoke
//! pattern so a thin sandbox doesn't surface as a false negative.

use std::path::PathBuf;

use heimdall_core::{Artifact, ArtifactKind};
use heimdall_disasm::{DisasmOpts, Disassembler, Isa, LlvmObjdumpDisassembler};

fn objdump_binary() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("HEIMDALL_LLVM_OBJDUMP_BIN") {
        let path = PathBuf::from(p);
        if path.is_file() {
            return Some(path);
        }
    }
    if let Ok(out) = std::process::Command::new("which")
        .arg("llvm-objdump")
        .output()
    {
        if out.status.success() {
            let p = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// `addi a0, x0, 0x42; ebreak` as raw RV64 LE bytes. Same payload the
/// fuzz raw-bytes-to-elf wrapper uses in its own tests, so this test
/// is comparing apples to apples with the rest of the pipeline.
const RV64_HELLO: &[u8] = &[
    0x13, 0x05, 0x20, 0x04, // addi a0, zero, 0x42
    0x73, 0x00, 0x10, 0x00, // ebreak
];

#[tokio::test]
async fn raw_bytes_disasm_against_real_llvm_objdump() {
    let Some(bin) = objdump_binary() else {
        eprintln!("llvm-objdump not found; skipping");
        return;
    };
    let d = LlvmObjdumpDisassembler::new(bin);
    let artifact = Artifact::new(ArtifactKind::RawBytes, RV64_HELLO.to_vec());
    let opts = DisasmOpts {
        isa: Isa::Rv64,
        raw_base_addr: 0x10000,
        extensions: Vec::new(),
    };
    let listing = d.disassemble(&artifact, &opts).await.expect("disassemble");

    assert_eq!(listing.isa, Isa::Rv64);
    assert!(!listing.lines.is_empty(), "must produce >=1 disasm line");
    // First instruction lands at the configured base address.
    assert_eq!(
        listing.lines[0].addr, 0x10000,
        "first instruction must be at raw_base_addr; got {:#x}",
        listing.lines[0].addr,
    );
    // li a0, 0x42 must show as either `li` (objdump pseudo) or `addi`
    // (canonical encoding). Both are correct decoders of the bytes.
    let first = &listing.lines[0];
    assert!(
        first.mnemonic == "li" || first.mnemonic == "addi",
        "first mnemonic must be li/addi; got `{}`",
        first.mnemonic,
    );
    // Look for an ebreak somewhere in the listing. Some llvm-objdump
    // versions emit it as a single mnemonic, others as `ebreak` with
    // no operands - either way the bytes should round-trip.
    assert!(
        listing.lines.iter().any(|l| l.mnemonic == "ebreak"),
        "expected an `ebreak` mnemonic in the listing; got: {:?}",
        listing
            .lines
            .iter()
            .map(|l| l.mnemonic.as_str())
            .collect::<Vec<_>>(),
    );
}

#[tokio::test]
async fn line_index_for_addr_locates_observed_pc() {
    let Some(bin) = objdump_binary() else {
        eprintln!("llvm-objdump not found; skipping");
        return;
    };
    let d = LlvmObjdumpDisassembler::new(bin);
    let artifact = Artifact::new(ArtifactKind::RawBytes, RV64_HELLO.to_vec());
    let listing = d
        .disassemble(&artifact, &DisasmOpts::default())
        .await
        .expect("disassemble");
    // Observed pc = base + 4 lands on the ebreak.
    let idx = listing.line_index_for_addr(0x10004).expect("pc must match");
    assert_eq!(listing.lines[idx].addr, 0x10004);
    assert!(listing.line_index_for_addr(0xdeadbeef).is_none());
}

/// M-extension instructions decode as `mul`/`div`/etc. when the
/// caller declares M is enabled.
#[tokio::test]
async fn mext_decodes_when_extension_enabled() {
    let Some(bin) = objdump_binary() else {
        eprintln!("llvm-objdump not found; skipping");
        return;
    };
    // `mul a0, a1, a2` LE-encoded.
    let bytes: &[u8] = &[0x33, 0x85, 0xc5, 0x02];
    let d = LlvmObjdumpDisassembler::new(bin);
    let artifact = Artifact::new(ArtifactKind::RawBytes, bytes.to_vec());
    let opts = DisasmOpts {
        extensions: vec!["m".into()],
        ..DisasmOpts::default()
    };
    let listing = d.disassemble(&artifact, &opts).await.expect("disassemble");
    let first = &listing.lines[0];
    assert_eq!(
        first.mnemonic, "mul",
        "M-ext mul must decode as `mul` when M is enabled (got `{}`)",
        first.mnemonic,
    );
}

/// Strictness contract: the same `mul` bytes must NOT decode when
/// M isn't in the enabled-extensions list. The point of this is
/// that an `<unknown>` row in the disasm UI is a real signal that
/// codegen emitted something the DUT can't execute, not a
/// disassembler misconfiguration we masked over with a permissive
/// `--mattr` superset.
#[tokio::test]
async fn mext_stays_unknown_when_extension_omitted() {
    let Some(bin) = objdump_binary() else {
        eprintln!("llvm-objdump not found; skipping");
        return;
    };
    let bytes: &[u8] = &[0x33, 0x85, 0xc5, 0x02]; // `mul a0, a1, a2`
    let d = LlvmObjdumpDisassembler::new(bin);
    let artifact = Artifact::new(ArtifactKind::RawBytes, bytes.to_vec());
    // Empty extensions list -> base I only.
    let listing = d
        .disassemble(&artifact, &DisasmOpts::default())
        .await
        .expect("disassemble");
    let first = &listing.lines[0];
    assert_ne!(
        first.mnemonic, "mul",
        "mul must NOT decode when M is absent; the strictness is the feature"
    );
}

#[tokio::test]
async fn missing_binary_surfaces_clean_error() {
    let d = LlvmObjdumpDisassembler::new("/this/path/definitely/does/not/exist/llvm-objdump");
    let artifact = Artifact::new(ArtifactKind::RawBytes, RV64_HELLO.to_vec());
    let err = d
        .disassemble(&artifact, &DisasmOpts::default())
        .await
        .expect_err("missing binary must error, not pretend to succeed");
    assert!(
        matches!(err, heimdall_disasm::DisasmError::ObjdumpNotFound { .. }),
        "wrong variant: {err:?}",
    );
}
