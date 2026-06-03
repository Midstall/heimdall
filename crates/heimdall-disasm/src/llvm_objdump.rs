//! Disassembler implementation that shells out to `llvm-objdump`.
//!
//! llvm-objdump only operates on real object files (it has no `-b
//! binary` raw-blob mode the way GNU objdump does), and it only
//! disassembles content that lives inside a SECTION header. The
//! [`heimdall_tools::raw_rv64_elf`] wrapper used by the fuzz path
//! emits a section-less ELF (its only sections are `null` + an empty
//! `.shstrtab`) because that satisfies spike's fesvr loader, which
//! walks PT_LOAD instead of sections. Feeding that to llvm-objdump
//! produces an empty listing because there's no SHT_PROGBITS section
//! covering the code.
//!
//! So this module carries its OWN minimal-ELF wrapper geared for
//! llvm-objdump: a single `.text` SHT_PROGBITS section flagged
//! `ALLOC|EXECINSTR` whose `sh_addr` is the caller-supplied
//! [`DisasmOpts::raw_base_addr`], paired with a PT_LOAD pointing at
//! the same offset. ELF inputs are passed through unchanged so a
//! real bring-up firmware renders at its linked addresses.
//!
//! The binary path defaults to `llvm-objdump` (PATH-resolved) but can
//! be overridden so the daemon can pin a specific nix-store path in
//! its config without leaking that detail into the trait.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::Stdio;

use async_trait::async_trait;
use heimdall_core::{Artifact, ArtifactKind};
use tempfile::NamedTempFile;
use tokio::process::Command;
use tracing::instrument;

use crate::error::DisasmError;
use crate::listing::{DisasmLine, DisasmListing};
use crate::trait_def::{DisasmOpts, Disassembler, Result};

/// Disassembler backed by an external `llvm-objdump` binary.
pub struct LlvmObjdumpDisassembler {
    binary: PathBuf,
}

impl LlvmObjdumpDisassembler {
    /// Construct against an explicit binary path. The daemon's config
    /// should pass the nix-store-resolved path; ad-hoc CLI tooling can
    /// pass `"llvm-objdump"` to use the PATH lookup.
    pub fn new(binary: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
        }
    }

    /// PATH-resolved default. Useful for tests + the
    /// `heimdall doctor`-style probes where the user has the tool
    /// installed somewhere on PATH.
    pub fn from_path() -> Self {
        Self::new("llvm-objdump")
    }
}

#[async_trait]
impl Disassembler for LlvmObjdumpDisassembler {
    fn name(&self) -> &str {
        "llvm-objdump"
    }

    fn supports(&self, kind: ArtifactKind) -> bool {
        matches!(kind, ArtifactKind::ElfRiscv | ArtifactKind::RawBytes)
    }

    #[instrument(skip(self, artifact, opts), fields(kind = ?artifact.kind))]
    async fn disassemble(&self, artifact: &Artifact, opts: &DisasmOpts) -> Result<DisasmListing> {
        if !self.supports(artifact.kind.clone()) {
            return Err(DisasmError::UnsupportedArtifact {
                disassembler: "llvm-objdump",
                kind: artifact.kind.clone(),
            });
        }
        // For RawBytes we wrap into a minimal ELF with a SHT_PROGBITS
        // `.text` section so llvm-objdump has something to disassemble.
        let bytes_to_disasm: std::borrow::Cow<'_, [u8]> = match &artifact.kind {
            ArtifactKind::ElfRiscv => std::borrow::Cow::Borrowed(&artifact.bytes),
            ArtifactKind::RawBytes => {
                let _ = opts.isa; // ISA pinning today is RV64-only; wrapper hard-codes RV64.
                std::borrow::Cow::Owned(wrap_rv64_le_for_objdump(
                    &artifact.bytes,
                    opts.raw_base_addr,
                ))
            }
            _ => unreachable!("supports() gate above"),
        };
        let mut file = NamedTempFile::new()?;
        file.write_all(&bytes_to_disasm)?;
        let path = file.into_temp_path();

        let mut cmd = Command::new(&self.binary);
        cmd.arg("-d");
        // Tell llvm-objdump exactly which extensions the DUT has
        // enabled. Strict-on-purpose: a `mul` decoding as `<unknown>`
        // means codegen emitted an M-ext instruction on a non-M
        // part, which is the bug we want surfaced. Skip the flag
        // entirely when the extension list is empty so we get the
        // ISA's default base behavior.
        if !opts.extensions.is_empty() {
            let mattr: String = opts
                .extensions
                .iter()
                .map(|e| format!("+{e}"))
                .collect::<Vec<_>>()
                .join(",");
            cmd.arg(format!("--mattr={mattr}"));
        }
        cmd.arg(&path);
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let output = cmd.output().await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                DisasmError::ObjdumpNotFound {
                    path: self.binary.display().to_string(),
                }
            } else {
                DisasmError::Io(e)
            }
        })?;
        if !output.status.success() {
            return Err(DisasmError::ObjdumpBadExit {
                status: output.status.code().unwrap_or(-1),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let lines = parse_objdump_output(&stdout)?;
        Ok(DisasmListing {
            name: "llvm-objdump".into(),
            isa: opts.isa,
            lines,
        })
    }
}

/// Parse `llvm-objdump -d` / `-D` text output into a flat
/// `Vec<DisasmLine>`.
///
/// Lines we care about look like:
///   `   10000: 93 00 00 00      addi    ra, zero, 0`
///   `   10000: 93000000         addi    ra, zero, 0`   (no-space form)
///   `   10004: 02 01            c.add    sp, t0`         (compressed)
///
/// Header / section / symbol lines that aren't disassembly are
/// skipped. The parser is forgiving: it tolerates 1..=4-byte
/// instructions (covering both RV32C compressed and RV64I), variable
/// whitespace, and the no-space hex form some llvm-objdump versions
/// emit when the line is short. It rejects (returns
/// [`DisasmError::ParseError`]) only when EVERY line in a file fails
/// to match, since that signals a format we never expected.
pub fn parse_objdump_output(text: &str) -> Result<Vec<DisasmLine>> {
    let mut lines = Vec::new();
    let mut any_skipped_non_disasm = false;
    for raw_line in text.lines() {
        // Strip leading whitespace.
        let line = raw_line.trim_start();
        // Lines we keep have the form `<hex-addr>:<tab><bytes><tab><mnemonic>[<tab><operands>]`.
        let Some((addr_str, rest)) = line.split_once(':') else {
            any_skipped_non_disasm = true;
            continue;
        };
        if addr_str.is_empty() || !addr_str.chars().all(|c| c.is_ascii_hexdigit()) {
            any_skipped_non_disasm = true;
            continue;
        }
        let Ok(addr) = u64::from_str_radix(addr_str, 16) else {
            any_skipped_non_disasm = true;
            continue;
        };
        // After the colon: leading whitespace, then hex bytes (possibly
        // space-separated or run together), then whitespace, then
        // mnemonic, then optional operands.
        let rest = rest.trim_start();
        let (bytes_part, after_bytes) = split_at_first_word_boundary(rest);
        let Some(bytes) = parse_byte_hex_run(bytes_part) else {
            any_skipped_non_disasm = true;
            continue;
        };
        let after_bytes = after_bytes.trim_start();
        let (mnemonic, operands_part) = match after_bytes.split_once(char::is_whitespace) {
            Some((m, ops)) => (m.to_string(), ops.trim().to_string()),
            None => (after_bytes.to_string(), String::new()),
        };
        if mnemonic.is_empty() {
            any_skipped_non_disasm = true;
            continue;
        }
        lines.push(DisasmLine {
            addr,
            bytes,
            mnemonic,
            operands: operands_part,
        });
    }
    if lines.is_empty() {
        return Err(DisasmError::ParseError {
            reason: format!(
                "no disassembly lines recognised (input had {} characters, \
                 saw_non_disasm_lines={})",
                text.len(),
                any_skipped_non_disasm,
            ),
        });
    }
    Ok(lines)
}

/// Split `s` at the first run of whitespace, returning (`head`, `tail`).
/// We can't just `split_whitespace` because the bytes block itself
/// contains internal whitespace (`93 00 00 00`); but the BOUNDARY
/// between bytes-block and mnemonic is always a tab on every
/// llvm-objdump emit I have notes for. Detect that by walking until
/// we hit a tab, OR until we hit a non-hex non-space character (the
/// mnemonic's first letter).
fn split_at_first_word_boundary(s: &str) -> (&str, &str) {
    let mut last_byte_end = 0usize;
    for (i, c) in s.char_indices() {
        if c == '\t' {
            return (&s[..i], &s[i + 1..]);
        }
        if c.is_ascii_hexdigit() || c == ' ' {
            if c.is_ascii_hexdigit() {
                last_byte_end = i + c.len_utf8();
            }
            continue;
        }
        // First non-hex non-space char: start of mnemonic. The byte
        // block ends at the last hex char we saw.
        return (&s[..last_byte_end], &s[i..]);
    }
    (s, "")
}

/// Parse a sequence of 1..=4 hex bytes from a string. The bytes can
/// be space-separated (`93 00 00 00`) or run together
/// (`93000000`). Returns `None` if the run has an odd count of hex
/// characters or is empty.
fn parse_byte_hex_run(s: &str) -> Option<Vec<u8>> {
    let stripped: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if stripped.is_empty() || stripped.len() % 2 != 0 {
        return None;
    }
    if !stripped.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let mut bytes = Vec::with_capacity(stripped.len() / 2);
    for chunk in stripped.as_bytes().chunks(2) {
        let s = std::str::from_utf8(chunk).ok()?;
        bytes.push(u8::from_str_radix(s, 16).ok()?);
    }
    Some(bytes)
}

/// Build a minimal RV64 little-endian ELF llvm-objdump can disassemble.
///
/// Layout: EHDR (0x40) + PHDR (0x38) + code + `.text` SHDR + `.shstrtab`
/// payload + 3 SHDRs (null, `.text`, `.shstrtab`). The `.text` section
/// is SHT_PROGBITS, `SHF_ALLOC | SHF_EXECINSTR`, `sh_addr = load_addr`,
/// `sh_offset = payload_off`, `sh_size = code.len()`. That's enough
/// for `llvm-objdump -d` to print "Disassembly of section .text:" at
/// the configured base address.
fn wrap_rv64_le_for_objdump(code: &[u8], load_addr: u64) -> Vec<u8> {
    const EHDR: usize = 0x40;
    const PHDR: usize = 0x38;
    const SHDR: usize = 0x40;
    let payload_off = EHDR + PHDR;
    // String table contains "\0.text\0.shstrtab\0".
    const SHSTRTAB: &[u8] = b"\0.text\0.shstrtab\0";
    let text_name: u32 = 1; // offset of ".text" in SHSTRTAB
    let shstrtab_name: u32 = 7; // offset of ".shstrtab"
    let shstrtab_off = payload_off + code.len();
    let shstrtab_size = SHSTRTAB.len();
    let shdr_off = shstrtab_off + shstrtab_size;
    let total = shdr_off + 3 * SHDR;
    let mut buf = vec![0u8; total];

    buf[0..4].copy_from_slice(b"\x7fELF");
    buf[4] = 2; // ELFCLASS64
    buf[5] = 1; // ELFDATA2LSB
    buf[6] = 1; // EI_VERSION
    buf[16..18].copy_from_slice(&2u16.to_le_bytes()); // ET_EXEC
    buf[18..20].copy_from_slice(&243u16.to_le_bytes()); // EM_RISCV
    buf[20..24].copy_from_slice(&1u32.to_le_bytes()); // e_version
    buf[24..32].copy_from_slice(&load_addr.to_le_bytes()); // e_entry
    buf[32..40].copy_from_slice(&(EHDR as u64).to_le_bytes()); // e_phoff
    buf[40..48].copy_from_slice(&(shdr_off as u64).to_le_bytes()); // e_shoff
    buf[52..54].copy_from_slice(&(EHDR as u16).to_le_bytes()); // e_ehsize
    buf[54..56].copy_from_slice(&(PHDR as u16).to_le_bytes()); // e_phentsize
    buf[56..58].copy_from_slice(&1u16.to_le_bytes()); // e_phnum
    buf[58..60].copy_from_slice(&(SHDR as u16).to_le_bytes()); // e_shentsize
    buf[60..62].copy_from_slice(&3u16.to_le_bytes()); // e_shnum
    buf[62..64].copy_from_slice(&2u16.to_le_bytes()); // e_shstrndx = shdr[2]

    let ph = EHDR;
    buf[ph..ph + 4].copy_from_slice(&1u32.to_le_bytes()); // PT_LOAD
    buf[ph + 4..ph + 8].copy_from_slice(&5u32.to_le_bytes()); // R+X
    buf[ph + 8..ph + 16].copy_from_slice(&(payload_off as u64).to_le_bytes());
    buf[ph + 16..ph + 24].copy_from_slice(&load_addr.to_le_bytes());
    buf[ph + 24..ph + 32].copy_from_slice(&load_addr.to_le_bytes());
    buf[ph + 32..ph + 40].copy_from_slice(&(code.len() as u64).to_le_bytes());
    buf[ph + 40..ph + 48].copy_from_slice(&(code.len() as u64).to_le_bytes());
    buf[ph + 48..ph + 56].copy_from_slice(&0x1000u64.to_le_bytes());

    buf[payload_off..payload_off + code.len()].copy_from_slice(code);
    buf[shstrtab_off..shstrtab_off + shstrtab_size].copy_from_slice(SHSTRTAB);

    // shdr[0]: null (all zero, already set).
    // shdr[1]: .text
    let sh1 = shdr_off + SHDR;
    buf[sh1..sh1 + 4].copy_from_slice(&text_name.to_le_bytes()); // sh_name
    buf[sh1 + 4..sh1 + 8].copy_from_slice(&1u32.to_le_bytes()); // SHT_PROGBITS
    buf[sh1 + 8..sh1 + 16].copy_from_slice(&0x6u64.to_le_bytes()); // ALLOC|EXECINSTR
    buf[sh1 + 16..sh1 + 24].copy_from_slice(&load_addr.to_le_bytes()); // sh_addr
    buf[sh1 + 24..sh1 + 32].copy_from_slice(&(payload_off as u64).to_le_bytes()); // sh_offset
    buf[sh1 + 32..sh1 + 40].copy_from_slice(&(code.len() as u64).to_le_bytes()); // sh_size
    buf[sh1 + 48..sh1 + 56].copy_from_slice(&4u64.to_le_bytes()); // sh_addralign
    // shdr[2]: .shstrtab
    let sh2 = shdr_off + 2 * SHDR;
    buf[sh2..sh2 + 4].copy_from_slice(&shstrtab_name.to_le_bytes());
    buf[sh2 + 4..sh2 + 8].copy_from_slice(&3u32.to_le_bytes()); // SHT_STRTAB
    buf[sh2 + 24..sh2 + 32].copy_from_slice(&(shstrtab_off as u64).to_le_bytes());
    buf[sh2 + 32..sh2 + 40].copy_from_slice(&(shstrtab_size as u64).to_le_bytes());
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verbatim snippet of `llvm-objdump -d` against an RV64 ELF on the
    /// nix-pinned llvm-21 build. Covers the canonical form: section
    /// header, symbol header, hex bytes with internal spaces, tab
    /// separator before mnemonic.
    const SAMPLE_OBJDUMP_OUTPUT: &str = "\
disasm.elf:\tfile format elf64-littleriscv

Disassembly of section .text:

0000000000010000 <_start>:
   10000: 93 00 00 00  \taddi\tra, zero, 0
   10004: 13 01 00 00  \taddi\tsp, zero, 0
   10008: 73 00 10 00  \tebreak
";

    #[test]
    fn parses_canonical_output() {
        let lines = parse_objdump_output(SAMPLE_OBJDUMP_OUTPUT).unwrap();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].addr, 0x10000);
        assert_eq!(lines[0].bytes, vec![0x93, 0x00, 0x00, 0x00]);
        assert_eq!(lines[0].mnemonic, "addi");
        assert_eq!(lines[0].operands, "ra, zero, 0");
        assert_eq!(lines[2].addr, 0x10008);
        assert_eq!(lines[2].mnemonic, "ebreak");
        assert!(lines[2].operands.is_empty(), "ebreak takes no operands");
    }

    /// llvm-objdump on some builds collapses the byte block to a
    /// no-space form when the bytes fit in a single 4-byte chunk:
    /// `   10000: 93000000     \taddi\tra, zero, 0`
    /// Parser must accept both forms.
    #[test]
    fn parses_no_space_bytes_form() {
        let text = "   10000: 93000000     \taddi\tra, zero, 0\n";
        let lines = parse_objdump_output(text).unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].bytes, vec![0x93, 0x00, 0x00, 0x00]);
        assert_eq!(lines[0].mnemonic, "addi");
    }

    /// Compressed (RVC) instructions are 2 bytes wide. Parser must
    /// accept them and not assume 4-byte width.
    #[test]
    fn parses_compressed_two_byte_instruction() {
        let text = "   10004: 02 01        \tc.add\tsp, t0\n";
        let lines = parse_objdump_output(text).unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].bytes, vec![0x02, 0x01]);
        assert_eq!(lines[0].mnemonic, "c.add");
        assert_eq!(lines[0].operands, "sp, t0");
    }

    #[test]
    fn skips_header_lines() {
        let text = "\
disasm.elf:\tfile format elf64-littleriscv

Disassembly of section .text:

0000000000010000 <_start>:
   10000: 93 00 00 00  \taddi\tra, zero, 0
";
        let lines = parse_objdump_output(text).unwrap();
        assert_eq!(lines.len(), 1, "only the actual disasm line should land");
    }

    /// Empty input or all-noise input must surface as a hard
    /// ParseError. Silent empty listings would let an
    /// llvm-objdump-version mismatch quietly nerf the UI.
    #[test]
    fn empty_output_is_a_parse_error() {
        let err = parse_objdump_output("").unwrap_err();
        assert!(matches!(err, DisasmError::ParseError { .. }));
        let err = parse_objdump_output("garbage\nlines\n").unwrap_err();
        assert!(matches!(err, DisasmError::ParseError { .. }));
    }

    #[test]
    fn byte_run_rejects_odd_hex_count() {
        assert!(parse_byte_hex_run("abc").is_none());
        assert!(parse_byte_hex_run("").is_none());
    }

    #[test]
    fn byte_run_accepts_space_or_run_together() {
        assert_eq!(parse_byte_hex_run("93 00").unwrap(), vec![0x93, 0x00]);
        assert_eq!(parse_byte_hex_run("9300").unwrap(), vec![0x93, 0x00]);
    }
}
