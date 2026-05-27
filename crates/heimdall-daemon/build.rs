//! Assembles the web-UI `assets/` directory under `$OUT_DIR` so `rust-embed`
//! can include both the hand-written static files (CSS, JS) and the SVGs
//! produced by the `heimdall-logo` Python package.
//!
//! Two paths for the SVGs:
//!
//! 1. `HEIMDALL_LOGO_SVGS` env var points at a directory of pre-rendered
//!    SVGs (the Nix path: `pkgs/heimdall-logo` has a `passthru.svgs`
//!    derivation that publishes them). Files are copied verbatim.
//!
//! 2. No env var set. Falls back to invoking `python3 -m heimdall_logo` with
//!    `PYTHONPATH` pointing at `../../pkgs/heimdall-logo`. This is the
//!    developer cargo-build path; requires `python3` on `PATH`.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR set by cargo"));
    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR set by cargo"));
    let assets_out = out_dir.join("assets");
    fs::create_dir_all(&assets_out).expect("create assets dir");

    copy_static_assets(&manifest_dir.join("assets"), &assets_out);
    render_logo_svgs(&manifest_dir, &assets_out);
}

fn copy_static_assets(src: &Path, dest: &Path) {
    println!("cargo:rerun-if-changed={}", src.display());
    for entry in fs::read_dir(src).expect("read assets dir") {
        let entry = entry.expect("entry");
        let path = entry.path();
        if path.is_file() {
            let name = path.file_name().expect("file name");
            fs::copy(&path, dest.join(name))
                .unwrap_or_else(|e| panic!("copy {}: {e}", path.display()));
        }
    }
}

fn render_logo_svgs(manifest_dir: &Path, dest: &Path) {
    println!("cargo:rerun-if-env-changed=HEIMDALL_LOGO_SVGS");
    if let Some(svgs_dir) = env::var_os("HEIMDALL_LOGO_SVGS") {
        copy_from_render(Path::new(&svgs_dir), dest);
        return;
    }
    invoke_python_generator(manifest_dir, dest);
}

fn copy_from_render(src: &Path, dest: &Path) {
    // The heimdall-logo Nix derivation writes these specific filenames.
    let mapping = [
        ("heimdall-favicon.svg", "favicon.svg"),
        ("heimdall-logomark-darkbg.svg", "heimdall-logomark.svg"),
    ];
    for (input, output) in mapping {
        let from = src.join(input);
        let to = dest.join(output);
        fs::copy(&from, &to)
            .unwrap_or_else(|e| panic!("copy {} -> {}: {e}", from.display(), to.display()));
    }
}

fn invoke_python_generator(manifest_dir: &Path, dest: &Path) {
    let logo_pkg = manifest_dir.join("../../pkgs/heimdall-logo");
    let logo_pkg = logo_pkg.canonicalize().unwrap_or(logo_pkg);
    println!("cargo:rerun-if-changed={}", logo_pkg.display());

    run_python(
        &logo_pkg,
        &[
            "-m",
            "heimdall_logo",
            "favicon",
            "--output",
            dest.join("favicon.svg").to_str().expect("utf-8 path"),
        ],
    );
    run_python(
        &logo_pkg,
        &[
            "-m",
            "heimdall_logo",
            "logomark",
            "--background",
            "#1a1b26",
            "--output",
            dest.join("heimdall-logomark.svg")
                .to_str()
                .expect("utf-8 path"),
        ],
    );
}

fn run_python(pythonpath: &Path, args: &[&str]) {
    let status = Command::new("python3")
        .env("PYTHONPATH", pythonpath)
        .args(args)
        .status()
        .expect(
            "failed to spawn `python3`. Either install python3 and the heimdall_logo \
             package, or set HEIMDALL_LOGO_SVGS to a directory of pre-rendered SVGs.",
        );
    if !status.success() {
        panic!("python3 {:?} exited with status {status:?}", args);
    }
}
