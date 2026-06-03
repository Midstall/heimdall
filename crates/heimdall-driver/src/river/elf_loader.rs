//! Parse a precompiled RV ELF into the segments that should be written to
//! the DUT plus the entry-point PC. Used by `RiverCpuDriver::load` to drop
//! firmware at the right physical addresses and by `run` to seed `dpc`
//! before the resume.
//!
//! Bare-metal RV: linker scripts set LMA == VMA, so the `p_paddr` field is
//! the address the bytes must end up at. Segments with `p_filesz < p_memsz`
//! have a BSS tail that the loader is responsible for zero-filling.

use elf::ElfBytes;
use elf::abi::PT_LOAD;
use elf::endian::AnyEndian;

use crate::error::DriverError;

/// One PT_LOAD segment ready to be written to the DUT.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadableSegment {
    /// Physical load address (`p_paddr`). Bytes go here.
    pub paddr: u64,
    /// Concrete bytes copied out of the ELF (`p_filesz` worth). Caller is
    /// expected to write these verbatim to `paddr`.
    pub bytes: Vec<u8>,
    /// Number of trailing zero bytes the loader must materialise after
    /// `bytes` to satisfy `p_memsz`. Skipped for segments with no BSS.
    pub zero_tail: u64,
}

/// Parsed view of an RV ELF: PT_LOAD segments plus the entry PC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedElf {
    pub entry: u64,
    pub segments: Vec<LoadableSegment>,
}

/// Parse `bytes` as a RISC-V ELF and pull out every PT_LOAD segment.
/// Empty `p_filesz` segments are dropped, but their `p_memsz` tail still
/// shows up as `zero_tail` so a caller-side `write_mem` (or skip) can be
/// driven without a second pass over the ELF.
pub fn parse(bytes: &[u8]) -> Result<ParsedElf, DriverError> {
    let file = ElfBytes::<AnyEndian>::minimal_parse(bytes)
        .map_err(|e| DriverError::Protocol(format!("river boot-elf parse: {e}")))?;
    let phdrs = file
        .segments()
        .ok_or_else(|| DriverError::Protocol("river boot-elf has no program headers".into()))?;
    let mut segments = Vec::new();
    for ph in phdrs {
        if ph.p_type != PT_LOAD {
            continue;
        }
        let file_sz = ph.p_filesz as usize;
        let off = ph.p_offset as usize;
        let end = off.checked_add(file_sz).ok_or_else(|| {
            DriverError::Protocol("river boot-elf: segment offset overflow".into())
        })?;
        if end > bytes.len() {
            return Err(DriverError::Protocol(format!(
                "river boot-elf: segment offset {off}+{file_sz} exceeds file size {}",
                bytes.len()
            )));
        }
        let data = bytes[off..end].to_vec();
        let zero_tail = ph.p_memsz.saturating_sub(ph.p_filesz);
        // Skip purely-empty headers (no bytes AND no bss). PT_LOAD with
        // `memsz==filesz==0` shows up in some toolchain outputs and is a
        // no-op.
        if data.is_empty() && zero_tail == 0 {
            continue;
        }
        segments.push(LoadableSegment {
            paddr: ph.p_paddr,
            bytes: data,
            zero_tail,
        });
    }
    if segments.is_empty() {
        return Err(DriverError::Protocol(
            "river boot-elf has no loadable PT_LOAD segments".into(),
        ));
    }
    Ok(ParsedElf {
        entry: file.ehdr.e_entry,
        segments,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-build a minimal 64-bit little-endian RV ELF with one PT_LOAD
    /// segment: 4 bytes of code at paddr=vaddr=0x10000, entry=0x100b0.
    /// Used by both the parser unit tests and the integration tests so the
    /// shape stays in one place.
    pub(super) fn build_minimal_rv64_elf() -> Vec<u8> {
        // ELF64 header is 0x40 bytes, one program header is 0x38 bytes.
        // We put 4 bytes of payload right after the program header.
        let mut buf = vec![0u8; 0x40 + 0x38 + 4];
        // e_ident
        buf[0..4].copy_from_slice(b"\x7fELF");
        buf[4] = 2; // EI_CLASS = ELFCLASS64
        buf[5] = 1; // EI_DATA = ELFDATA2LSB
        buf[6] = 1; // EI_VERSION = 1
        // e_type = ET_EXEC (2), e_machine = EM_RISCV (243)
        buf[16..18].copy_from_slice(&2u16.to_le_bytes());
        buf[18..20].copy_from_slice(&243u16.to_le_bytes());
        // e_version
        buf[20..24].copy_from_slice(&1u32.to_le_bytes());
        // e_entry = 0x100b0
        buf[24..32].copy_from_slice(&0x100b0u64.to_le_bytes());
        // e_phoff = 0x40
        buf[32..40].copy_from_slice(&0x40u64.to_le_bytes());
        // e_shoff = 0 (no sections)
        // e_flags = 0
        // e_ehsize = 0x40
        buf[52..54].copy_from_slice(&0x40u16.to_le_bytes());
        // e_phentsize = 0x38, e_phnum = 1
        buf[54..56].copy_from_slice(&0x38u16.to_le_bytes());
        buf[56..58].copy_from_slice(&1u16.to_le_bytes());

        // Program header at offset 0x40.
        let ph = 0x40;
        // p_type = PT_LOAD (1)
        buf[ph..ph + 4].copy_from_slice(&1u32.to_le_bytes());
        // p_flags = R+X (5)
        buf[ph + 4..ph + 8].copy_from_slice(&5u32.to_le_bytes());
        // p_offset = 0x78 (right after the program header)
        buf[ph + 8..ph + 16].copy_from_slice(&0x78u64.to_le_bytes());
        // p_vaddr = 0x10000, p_paddr = 0x10000
        buf[ph + 16..ph + 24].copy_from_slice(&0x10000u64.to_le_bytes());
        buf[ph + 24..ph + 32].copy_from_slice(&0x10000u64.to_le_bytes());
        // p_filesz = 4, p_memsz = 4 (no bss)
        buf[ph + 32..ph + 40].copy_from_slice(&4u64.to_le_bytes());
        buf[ph + 40..ph + 48].copy_from_slice(&4u64.to_le_bytes());
        // p_align = 0x1000
        buf[ph + 48..ph + 56].copy_from_slice(&0x1000u64.to_le_bytes());

        // Payload: 4 magic bytes the test asserts on.
        buf[0x78..0x7c].copy_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD]);
        buf
    }

    #[test]
    fn parses_minimal_rv64_elf() {
        let bytes = build_minimal_rv64_elf();
        let parsed = parse(&bytes).expect("parse");
        assert_eq!(parsed.entry, 0x100b0);
        assert_eq!(parsed.segments.len(), 1);
        let seg = &parsed.segments[0];
        assert_eq!(seg.paddr, 0x10000);
        assert_eq!(seg.bytes, vec![0xAA, 0xBB, 0xCC, 0xDD]);
        assert_eq!(seg.zero_tail, 0);
    }

    #[test]
    fn rejects_non_elf_bytes() {
        let err = parse(b"not an elf").unwrap_err();
        assert!(matches!(err, DriverError::Protocol(_)));
    }

    #[test]
    fn segment_with_bss_tail_reports_zero_tail() {
        // Take the minimal ELF and bump the memsz so memsz > filesz.
        let mut bytes = build_minimal_rv64_elf();
        let ph = 0x40;
        // p_memsz = 16 (12 bytes of BSS after the 4 bytes of code).
        bytes[ph + 40..ph + 48].copy_from_slice(&16u64.to_le_bytes());
        let parsed = parse(&bytes).expect("parse");
        assert_eq!(parsed.segments[0].bytes.len(), 4);
        assert_eq!(parsed.segments[0].zero_tail, 12);
    }
}
