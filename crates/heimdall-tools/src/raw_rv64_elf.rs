//! Tool that wraps raw RV64 machine-code bytes in a minimal RV64 LE ELF.
//!
//! The fuzzer's `RawAsmGen<Rv64>` emits an `ArtifactKind::RawBytes` artifact
//! holding a pre-encoded instruction stream. The River driver's `load`
//! parses ELF and writes each PT_LOAD segment to its `p_paddr`, so the
//! toolchain needs a step that wraps those raw bytes in a single-segment
//! ELF with `entry == p_paddr` so `dpc = entry` lands on the first
//! generated instruction.
//!
//! The wrapped image targets `0x10000` to match the bring-up firmware's
//! link address. The address is fixed today; if multi-DUT layouts diverge
//! it can become a `ToolOpts` key.
//!
//! Used by [`crate::ToolChain`] under the `RiverFuzzFactory` path.

use async_trait::async_trait;
use heimdall_core::{Artifact, ArtifactKind};

use crate::trait_def::{Result, TargetSpec, Tool, ToolOpts};

/// Default load address for fuzz-generated programs. Matches the bring-up
/// firmware's linker script (`_start = 0x10000`).
pub const FUZZ_LOAD_ADDR: u64 = 0x10000;

pub struct RawBytesToElfRiscv;

impl RawBytesToElfRiscv {
    pub fn new() -> Self {
        Self
    }
}

impl Default for RawBytesToElfRiscv {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for RawBytesToElfRiscv {
    fn name(&self) -> &str {
        "raw-bytes-to-elf-riscv"
    }

    fn supports(&self, input: &Artifact, target: &TargetSpec) -> bool {
        matches!(input.kind, ArtifactKind::RawBytes)
            && matches!(target.desired_output, ArtifactKind::ElfRiscv)
    }

    async fn run(
        &self,
        input: &Artifact,
        _target: &TargetSpec,
        _opts: &ToolOpts,
    ) -> Result<Artifact> {
        let elf = wrap_rv64_le(&input.bytes, FUZZ_LOAD_ADDR);
        Ok(Artifact::new(ArtifactKind::ElfRiscv, elf))
    }

    fn version_fingerprint(&self) -> String {
        format!("raw-bytes-to-elf-riscv:1:load={FUZZ_LOAD_ADDR:#x}")
    }
}

/// Build a minimal RV64 little-endian ELF with one PT_LOAD segment holding
/// `code` at `load_addr`, entry == `load_addr`. Matches the structure the
/// River driver's `elf_loader::parse` expects.
///
/// Section table layout:
/// - shdr[0]: SHN_UNDEF null section (required by ELF spec).
/// - shdr[1]: `.shstrtab` (SHT_STRTAB) of size 1 holding a single NUL byte.
///
/// Two SHDRs (not one) because spike's fesvr loader asserts both
/// `e_shstrndx < e_shnum` AND `sh[i].sh_name < sh[e_shstrndx].sh_size`
/// for every section. A solo null SHDR satisfies the first but trips
/// the second (sh_size=0). The single-NUL string table makes every
/// `sh_name=0` reference a valid (empty) string and unblocks load.
fn wrap_rv64_le(code: &[u8], load_addr: u64) -> Vec<u8> {
    const EHDR: usize = 0x40;
    const PHDR: usize = 0x38;
    const SHDR: usize = 0x40;
    let payload_off = EHDR + PHDR;
    let shstrtab_off = payload_off + code.len();
    let shstrtab_size: usize = 1; // a single '\0'
    let shdr_off = shstrtab_off + shstrtab_size;
    let total = shdr_off + 2 * SHDR;
    let mut buf = vec![0u8; total];

    // e_ident
    buf[0..4].copy_from_slice(b"\x7fELF");
    buf[4] = 2; // EI_CLASS = ELFCLASS64
    buf[5] = 1; // EI_DATA  = ELFDATA2LSB
    buf[6] = 1; // EI_VERSION
    // e_type = ET_EXEC, e_machine = EM_RISCV (243)
    buf[16..18].copy_from_slice(&2u16.to_le_bytes());
    buf[18..20].copy_from_slice(&243u16.to_le_bytes());
    buf[20..24].copy_from_slice(&1u32.to_le_bytes()); // e_version
    buf[24..32].copy_from_slice(&load_addr.to_le_bytes()); // e_entry
    buf[32..40].copy_from_slice(&(EHDR as u64).to_le_bytes()); // e_phoff
    buf[40..48].copy_from_slice(&(shdr_off as u64).to_le_bytes()); // e_shoff
    buf[52..54].copy_from_slice(&(EHDR as u16).to_le_bytes()); // e_ehsize
    buf[54..56].copy_from_slice(&(PHDR as u16).to_le_bytes()); // e_phentsize
    buf[56..58].copy_from_slice(&1u16.to_le_bytes()); // e_phnum
    buf[58..60].copy_from_slice(&(SHDR as u16).to_le_bytes()); // e_shentsize
    buf[60..62].copy_from_slice(&2u16.to_le_bytes()); // e_shnum (null + shstrtab)
    buf[62..64].copy_from_slice(&1u16.to_le_bytes()); // e_shstrndx = shdr[1]

    // Program header at offset EHDR.
    let ph = EHDR;
    buf[ph..ph + 4].copy_from_slice(&1u32.to_le_bytes()); // p_type = PT_LOAD
    buf[ph + 4..ph + 8].copy_from_slice(&5u32.to_le_bytes()); // p_flags = R+X
    buf[ph + 8..ph + 16].copy_from_slice(&(payload_off as u64).to_le_bytes()); // p_offset
    buf[ph + 16..ph + 24].copy_from_slice(&load_addr.to_le_bytes()); // p_vaddr
    buf[ph + 24..ph + 32].copy_from_slice(&load_addr.to_le_bytes()); // p_paddr
    buf[ph + 32..ph + 40].copy_from_slice(&(code.len() as u64).to_le_bytes()); // p_filesz
    buf[ph + 40..ph + 48].copy_from_slice(&(code.len() as u64).to_le_bytes()); // p_memsz
    buf[ph + 48..ph + 56].copy_from_slice(&0x1000u64.to_le_bytes()); // p_align

    buf[payload_off..payload_off + code.len()].copy_from_slice(code);

    // .shstrtab contents: one NUL byte. buf is already zeroed so the
    // byte at shstrtab_off is already '\0'.
    let _ = shstrtab_off; // explicit reference for the reader.

    // shdr[0]: SHN_UNDEF null section (all fields zero, already set).
    // shdr[1]: .shstrtab.
    let sh1 = shdr_off + SHDR;
    buf[sh1..sh1 + 4].copy_from_slice(&0u32.to_le_bytes()); // sh_name = 0
    buf[sh1 + 4..sh1 + 8].copy_from_slice(&3u32.to_le_bytes()); // sh_type = SHT_STRTAB
    buf[sh1 + 8..sh1 + 16].copy_from_slice(&0u64.to_le_bytes()); // sh_flags
    buf[sh1 + 16..sh1 + 24].copy_from_slice(&0u64.to_le_bytes()); // sh_addr
    buf[sh1 + 24..sh1 + 32].copy_from_slice(&(shstrtab_off as u64).to_le_bytes()); // sh_offset
    buf[sh1 + 32..sh1 + 40].copy_from_slice(&(shstrtab_size as u64).to_le_bytes()); // sh_size
    // sh_link, sh_info, sh_addralign, sh_entsize all zero.
    buf
}

#[cfg(test)]
mod tests {
    use super::*;
    use heimdall_core::DutKind;

    #[tokio::test]
    async fn wraps_raw_bytes_into_rv64_elf_at_fuzz_load_addr() {
        let tool = RawBytesToElfRiscv::new();
        let payload = vec![0x13u8, 0x05, 0x20, 0x04, 0x73, 0x00, 0x10, 0x00]; // li a0,0x42; ebreak
        let input = Artifact::new(ArtifactKind::RawBytes, payload.clone());
        let target = TargetSpec {
            dut_kind: DutKind::RiverRc1Nano,
            desired_output: ArtifactKind::ElfRiscv,
        };
        let opts = ToolOpts::default();
        assert!(tool.supports(&input, &target));
        let out = tool.run(&input, &target, &opts).await.unwrap();
        assert!(matches!(out.kind, ArtifactKind::ElfRiscv));
        // ELF magic survives.
        assert_eq!(&out.bytes[..4], b"\x7fELF");
        // e_entry == FUZZ_LOAD_ADDR (LE).
        let entry = u64::from_le_bytes(out.bytes[24..32].try_into().unwrap());
        assert_eq!(entry, FUZZ_LOAD_ADDR);
        // p_paddr == FUZZ_LOAD_ADDR.
        let paddr = u64::from_le_bytes(out.bytes[0x40 + 24..0x40 + 32].try_into().unwrap());
        assert_eq!(paddr, FUZZ_LOAD_ADDR);
        // Payload at the end is verbatim.
        let payload_off = 0x40 + 0x38;
        assert_eq!(
            &out.bytes[payload_off..payload_off + payload.len()],
            &payload[..]
        );
        // Section header table: null SHDR + .shstrtab SHDR so spike's
        // fesvr load_elf passes both `e_shstrndx < e_shnum` AND
        // `sh_name < sh[e_shstrndx].sh_size` for every section.
        let shstrtab_off = payload_off + payload.len();
        let shdr_off = shstrtab_off + 1; // 1 byte of .shstrtab payload
        let e_shoff = u64::from_le_bytes(out.bytes[40..48].try_into().unwrap());
        let e_shnum = u16::from_le_bytes(out.bytes[60..62].try_into().unwrap());
        let e_shentsize = u16::from_le_bytes(out.bytes[58..60].try_into().unwrap());
        let e_shstrndx = u16::from_le_bytes(out.bytes[62..64].try_into().unwrap());
        assert_eq!(e_shoff, shdr_off as u64);
        assert_eq!(e_shnum, 2);
        assert_eq!(e_shentsize, 0x40);
        assert_eq!(e_shstrndx, 1);
        assert!(e_shstrndx < e_shnum, "spike fesvr SHDR-count assertion");
        // .shstrtab byte is a single NUL.
        assert_eq!(out.bytes[shstrtab_off], 0);
        // shdr[0] is the all-zero null SHDR (SHN_UNDEF).
        assert_eq!(&out.bytes[shdr_off..shdr_off + 0x40], &[0u8; 0x40][..]);
        // shdr[1] is SHT_STRTAB with sh_offset=shstrtab_off, sh_size=1.
        let sh1 = shdr_off + 0x40;
        let sh_type = u32::from_le_bytes(out.bytes[sh1 + 4..sh1 + 8].try_into().unwrap());
        let sh_offset = u64::from_le_bytes(out.bytes[sh1 + 24..sh1 + 32].try_into().unwrap());
        let sh_size = u64::from_le_bytes(out.bytes[sh1 + 32..sh1 + 40].try_into().unwrap());
        assert_eq!(sh_type, 3, "SHT_STRTAB");
        assert_eq!(sh_offset, shstrtab_off as u64);
        assert_eq!(sh_size, 1);
        assert!(
            sh_size > 0,
            "spike fesvr sh_name assertion needs nonzero shstrtab"
        );
    }

    #[test]
    fn does_not_claim_elf_input() {
        let tool = RawBytesToElfRiscv::new();
        let input = Artifact::new(ArtifactKind::ElfRiscv, vec![0u8; 64]);
        let target = TargetSpec {
            dut_kind: DutKind::RiverRc1Nano,
            desired_output: ArtifactKind::ElfRiscv,
        };
        // Already-ElfRiscv inputs should pass straight through the chain
        // without this tool grabbing them.
        assert!(!tool.supports(&input, &target));
    }
}
