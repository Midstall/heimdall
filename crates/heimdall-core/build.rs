//! Compute the workspace's full version string.
//!
//! 1. Nix builds preset `HEIMDALL_FULL_VERSION` (see `flake.nix`'s
//!    `commonArgs`) using flakever's
//!    `<cargoVersion>pre-<lastModifiedDate>-<rev>` shape. We pass
//!    that through verbatim.
//! 2. Otherwise we synthesise the same shape from `git`.
//! 3. With no git history (e.g. a crates.io tarball), the var stays
//!    unset and `heimdall_core::VERSION`'s `option_env!` falls
//!    through to `CARGO_PKG_VERSION`.

use std::process::Command;

fn main() {
    // HEAD changes on commit/checkout, index changes on stage/unstage.
    // Together they cover everything that would flip the synthesised
    // rev or `-dirty` suffix.
    println!("cargo:rerun-if-env-changed=HEIMDALL_FULL_VERSION");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/index");

    if let Ok(forced) = std::env::var("HEIMDALL_FULL_VERSION") {
        if !forced.trim().is_empty() {
            println!("cargo:rustc-env=HEIMDALL_FULL_VERSION={forced}");
            return;
        }
    }

    let cargo_pkg = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    if let Some(synth) = synth_from_git(&cargo_pkg) {
        println!("cargo:rustc-env=HEIMDALL_FULL_VERSION={synth}");
    }
}

/// Build `<cargoVersion>pre-<YYYYMMDD>-<short-rev>[-dirty]` from
/// the surrounding git checkout. `None` means the caller should
/// fall back to the bare version.
fn synth_from_git(cargo_pkg: &str) -> Option<String> {
    let date = git_output(&["log", "-1", "--format=%cd", "--date=format:%Y%m%d"])?;
    let rev = git_output(&["rev-parse", "--short", "HEAD"])?;
    let dirty = git_output(&["status", "--porcelain"])
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    let suffix = if dirty { "-dirty" } else { "" };
    Some(format!("{cargo_pkg}pre-{date}-{rev}{suffix}"))
}

fn git_output(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}
