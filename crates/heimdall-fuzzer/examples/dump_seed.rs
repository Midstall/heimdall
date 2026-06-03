//! Dump the deterministic RawAsmGen-produced program bytes for a given
//! `(--seed, --iter)` pair, so the River counterpart (or anyone) can run
//! the exact same program directly on emu + spike outside the JTAG path
//! and classify divergences without re-driving the whole fuzz session.
//!
//! Layout mirrors `FuzzerEngine::run`'s state-consumption order so the
//! bytes match what a real fuzz run produces. The default scheduler is
//! `RoundRobinScheduler`. At iter 0 the corpus is empty so the choice is
//! always `GenerateFresh` and the body only depends on the engine seed.
//! For iter > 0 the choice can be `MutateAt(idx)`, which depends on
//! prior verdicts (corpus growth). We replicate the rng-consumption
//! pattern but not verdict-driven mutation, so iter > 0 here matches the
//! `--mock-mode` case where every iter is generate-fresh. For triage of
//! a single failing iter, run with `--iter <N>` and check the artifact
//! against the daemon's log.
//!
//! Build + run:
//!   cargo run -p heimdall-fuzzer --example dump_seed -- \
//!       --seed 77 --iter 0 --insn-count 16 --output /tmp/fuzz-seed-77.bin
//!
//! Output is raw bytes ready to wrap in an ELF (FUZZ_LOAD_ADDR=0x10000)
//! or feed straight into the emulator's `--memory ram` region.

use clap::Parser;
use heimdall_core::SeedId;
use heimdall_fuzzer::traits::Generator;
use heimdall_fuzzer::{EBREAK_INSN, RawAsmGen, Rv64};
use rand::RngCore;
use rand::SeedableRng;
use rand::rngs::StdRng;
use std::path::PathBuf;

#[derive(Debug, Parser)]
struct Args {
    /// Engine RNG seed (same value you pass to `heimdall fuzz --seed`).
    #[arg(long)]
    seed: u64,
    /// Which iteration to dump. iter 0 is always generate-fresh. Higher
    /// iterations replicate iter-0 generation logic per iter (does not
    /// model mutation-driven scheduling).
    #[arg(long, default_value_t = 0)]
    iter: u64,
    /// Body instruction count (same as `heimdall fuzz --insn-count`).
    #[arg(long, default_value_t = 16)]
    insn_count: usize,
    /// Where to write the raw bytes. `-` writes to stdout.
    #[arg(long, short = 'o', default_value = "-")]
    output: PathBuf,
    /// Suppress the trailing ebreak. Matches `--ebreak-terminator false`
    /// on the engine path.
    #[arg(long, default_value_t = false)]
    no_ebreak: bool,
    /// Suppress the 31-insn register-zeroing prologue. Mirrors
    /// `RawAsmGen::with_zero_init_prologue(false)` on the engine path.
    /// Use this when comparing against a model that re-zeroes GPRs
    /// externally, otherwise the default (on) is what fuzz runs see.
    #[arg(long, default_value_t = false)]
    no_prologue: bool,
    /// Also print a short hex summary to stderr so the bytes are inline-
    /// quotable on the bus.
    #[arg(long, default_value_t = true)]
    summary: bool,
}

fn main() -> std::io::Result<()> {
    let args = Args::parse();
    let mut rng = StdRng::seed_from_u64(args.seed);
    let mut generator = RawAsmGen::<Rv64>::new(args.insn_count)
        .with_ebreak_terminator(!args.no_ebreak)
        .with_zero_init_prologue(!args.no_prologue);

    // Replicate FuzzerEngine's per-iter consumption: one next_u64() for
    // the SeedId, then generator.generate(&mut rng, seed_id).
    let mut artifact_bytes: Vec<u8> = Vec::new();
    for _ in 0..=args.iter {
        let seed_id = SeedId(rng.next_u64());
        artifact_bytes = generator.generate(&mut rng, seed_id).bytes.to_vec();
    }

    if args.output.as_os_str() == "-" {
        use std::io::Write as _;
        std::io::stdout().write_all(&artifact_bytes)?;
    } else {
        std::fs::write(&args.output, &artifact_bytes)?;
        eprintln!(
            "wrote {} bytes to {}",
            artifact_bytes.len(),
            args.output.display()
        );
    }

    if args.summary {
        let head: Vec<String> = artifact_bytes
            .chunks_exact(4)
            .take(8)
            .map(|c| {
                let w = u32::from_le_bytes([c[0], c[1], c[2], c[3]]);
                format!("{w:08x}")
            })
            .collect();
        eprintln!(
            "len={} bytes, first 8 words: {}",
            artifact_bytes.len(),
            head.join(" ")
        );
        if !args.no_ebreak {
            let last = u32::from_le_bytes([
                artifact_bytes[artifact_bytes.len() - 4],
                artifact_bytes[artifact_bytes.len() - 3],
                artifact_bytes[artifact_bytes.len() - 2],
                artifact_bytes[artifact_bytes.len() - 1],
            ]);
            assert_eq!(last, EBREAK_INSN, "last word must be ebreak");
            eprintln!("trailing ebreak: 0x{last:08x}");
        }
    }
    Ok(())
}
