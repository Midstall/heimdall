//! Web-asset build: bundle + minify `frontend/src/*.ts` into a single
//! `app.js` and stage it (plus `styles.css` and rendered SVGs) under
//! `$OUT_DIR/assets/` for `rust-embed` to pick up.
//!
//! Pipeline: drive [`rolldown::Bundler`] over `frontend/src/main.ts`,
//! which transitively pulls in the rest of the modules via ES-module
//! resolution, runs the oxc transformer on each, and emits a single
//! minified bundle. No Node required.
//!
//! Type checking is intentionally NOT done here. Run `tsc --noEmit`
//! locally when you want it.
//!
//! SVGs: same two-path flow the previous heimdall-daemon build.rs
//! had. `HEIMDALL_LOGO_SVGS` pre-rendered dir overrides the in-tree
//! `pkgs/heimdall-logo` Python fallback.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use rolldown::{
    Bundler, BundlerOptions, InputItem, OutputFormat, RawMinifyOptions, ResolveOptions,
};

fn main() {
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR set by cargo"));
    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR set by cargo"));
    let assets_out = out_dir.join("assets");
    // Clear the assets dir so stale outputs from prior build.rs
    // versions (e.g. the per-module .js files an earlier pipeline
    // emitted) don't get embedded alongside the new bundle.
    if assets_out.exists() {
        fs::remove_dir_all(&assets_out).expect("clear assets dir");
    }
    fs::create_dir_all(&assets_out).expect("create assets dir");

    // ts-rs emits .ts files into `$OUT_DIR/bindings/` so the source
    // tree never holds generated code (no gitignore, no stale-copy
    // class of bug). The bundler resolves `import "bindings/Foo.js"`
    // by way of a resolve alias plugged into rolldown below.
    let bindings_dir = out_dir.join("bindings");
    generate_ts_bindings(&bindings_dir);
    bundle_frontend(&manifest_dir.join("frontend"), &assets_out, &bindings_dir);
    copy_styles(&manifest_dir.join("frontend/src"), &assets_out);
    stage_lucide(&assets_out);
    render_logo_svgs(&manifest_dir, &assets_out);
}

/// Drive ts-rs to emit a `.ts` file per wire type under
/// `$OUT_DIR/bindings/`. The previous design generated these via a
/// `cargo test --features bindings` target inside `heimdall-daemon`
/// and committed the output, which forced contributors to
/// remember to regenerate on every wire-type change. Doing it here
/// gates regeneration on actual build activity, and keeping the
/// destination under OUT_DIR means the source tree never holds
/// generated files - nothing to gitignore, nothing to accidentally
/// commit.
fn generate_ts_bindings(bindings_dir: &Path) {
    use ts_rs::{Config, TS};

    if bindings_dir.exists() {
        fs::remove_dir_all(bindings_dir).expect("clear stale bindings dir");
    }
    fs::create_dir_all(bindings_dir).expect("create bindings dir");
    // Direct ts-rs at the absolute bindings dir via env var so
    // every derive can just say `#[ts(export)]` and the filename
    // gets dropped into this directory, with no
    // `export_to = "..."` boilerplate on the wire types.
    // SAFETY: we're inside a build script (single-threaded, no
    // other threads can race on environment lookups), so the
    // unsafe contract on set_var (no concurrent reads) holds.
    unsafe {
        std::env::set_var("TS_RS_EXPORT_DIR", bindings_dir);
    }

    // from_env() picks up TS_RS_EXPORT_DIR (set just above) so the
    // bindings land in our absolute frontend dir. The chained
    // `with_*` calls then layer in the project-specific overrides:
    //   - large_int: u64/i64 ride as JS `number` inside safe range.
    //     `bigint` would force casts on every read for no gain.
    //   - import_extension: imports point at the swc-emitted `.js`
    //     siblings the browser fetches from /assets/.
    let cfg = Config::from_env()
        .with_large_int("number")
        .with_import_extension(Some("js"));

    use heimdall_api::{
        AboutInfo, CampaignState, CampaignTemplate, JobKind, JobState, LogLevel, SnapshotSourceTag,
        VerdictSummary,
    };
    use heimdall_core::{State, ValueRepr};
    use heimdall_disasm::{DisasmListing, Isa};

    JobKind::export_all(&cfg).expect("export JobKind + descendants");
    JobState::export_all(&cfg).expect("export JobState");
    VerdictSummary::export(&cfg).expect("export VerdictSummary");
    CampaignTemplate::export(&cfg).expect("export CampaignTemplate");
    CampaignState::export(&cfg).expect("export CampaignState");
    SnapshotSourceTag::export(&cfg).expect("export SnapshotSourceTag");
    LogLevel::export(&cfg).expect("export LogLevel");
    AboutInfo::export_all(&cfg).expect("export AboutInfo + AboutFeatures");
    State::export_all(&cfg).expect("export State + ValueRepr");
    ValueRepr::export(&cfg).expect("export ValueRepr");
    DisasmListing::export_all(&cfg).expect("export DisasmListing + DisasmLine");
    Isa::export(&cfg).expect("export Isa");
}

/// Bundle the TypeScript frontend rooted at `frontend/src/main.ts`
/// using rolldown. Output: a single minified `app.js` under
/// `out_dir`.
///
/// Imports of the form `bindings/Foo.js` are redirected to
/// `$OUT_DIR/bindings/` via a resolve alias, so generated TypeScript
/// definitions never need to live in the source tree.
fn bundle_frontend(frontend_dir: &Path, out_dir: &Path, bindings_dir: &Path) {
    let src_dir = frontend_dir.join("src");
    println!("cargo:rerun-if-changed={}", src_dir.display());
    println!("cargo:rerun-if-changed={}", bindings_dir.display());
    let bindings_abs = bindings_dir.to_string_lossy().into_owned();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime for rolldown");
    runtime.block_on(async {
        let options = BundlerOptions {
            input: Some(vec![InputItem {
                name: Some("app".to_string()),
                import: src_dir.join("main.ts").to_string_lossy().into_owned(),
            }]),
            cwd: Some(frontend_dir.to_path_buf()),
            dir: Some(out_dir.to_string_lossy().into_owned()),
            format: Some(OutputFormat::Iife),
            minify: Some(RawMinifyOptions::Bool(true)),
            entry_filenames: Some("[name].js".to_string().into()),
            chunk_filenames: Some("[name].js".to_string().into()),
            // `import "bindings/Foo.js"` -> `$OUT_DIR/bindings/Foo.ts`.
            // oxc-resolver picks the first replacement that resolves to
            // a real file, which is exactly what we want here.
            resolve: Some(ResolveOptions {
                alias: Some(vec![("bindings".to_string(), vec![Some(bindings_abs)])]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut bundler = Bundler::new(options).expect("rolldown construct bundler");
        let result = bundler.write().await.expect("rolldown bundle write");
        if !result.warnings.is_empty() {
            for w in &result.warnings {
                println!("cargo:warning=rolldown: {w:?}");
            }
        }
    });
}

fn copy_styles(src_dir: &Path, out_dir: &Path) {
    let css = src_dir.join("styles.css");
    if !css.exists() {
        return;
    }
    println!("cargo:rerun-if-changed={}", css.display());
    let dest = out_dir.join("app.css");
    fs::copy(&css, &dest).unwrap_or_else(|e| panic!("copy {}: {e}", css.display()));
}

/// Bake the Lucide icon set into the embedded assets.
///
/// We use a tiny subset of the icons (restart button, row toggle).
/// The full ttf is small enough (~80KB) that there's no point doing
/// a per-glyph subset pass. Ship the whole thing and let the
/// browser cache it once. The codepoints we use are looked up from
/// the `Icon` enum at build time so we never embed a hard-coded
/// `\eXXX` literal that could drift when the upstream font version
/// bumps.
fn stage_lucide(dest: &Path) {
    use lucide_icons::{Icon, LUCIDE_FONT_BYTES};

    let ttf = dest.join("lucide.ttf");
    fs::write(&ttf, LUCIDE_FONT_BYTES).unwrap_or_else(|e| panic!("write {}: {e}", ttf.display()));

    // (lucide-name, css-class-suffix). Add a row here when you
    // need a new icon in the UI. The generated CSS makes it
    // available as `.icon-<suffix>`.
    let icons: &[(&str, &str)] = &[
        ("rotate-cw", "restart"),
        ("chevron-right", "row-closed"),
        ("chevron-down", "row-open"),
        ("x", "cancel"),
    ];

    let mut css = String::with_capacity(1024);
    css.push_str(
        "/* Generated by heimdall-web/build.rs from lucide-icons at build time. */\n\
         @font-face {\n\
         \x20\x20font-family: 'Lucide';\n\
         \x20\x20src: url('/assets/lucide.ttf') format('truetype');\n\
         \x20\x20font-weight: normal;\n\
         \x20\x20font-style: normal;\n\
         \x20\x20font-display: block;\n\
         }\n\
         .lucide {\n\
         \x20\x20font-family: 'Lucide';\n\
         \x20\x20font-style: normal;\n\
         \x20\x20font-weight: normal;\n\
         \x20\x20font-variant: normal;\n\
         \x20\x20text-transform: none;\n\
         \x20\x20line-height: 1;\n\
         \x20\x20display: inline-block;\n\
         \x20\x20-webkit-font-smoothing: antialiased;\n\
         \x20\x20-moz-osx-font-smoothing: grayscale;\n\
         }\n",
    );
    for (name, class) in icons {
        let icon =
            Icon::try_from(*name).unwrap_or_else(|e| panic!("lucide icon `{name}` not found: {e}"));
        let cp = char::from(icon) as u32;
        // \xHH escapes can be ambiguous. Use the explicit 4-digit
        // form to avoid collisions with following hex chars.
        let _ = std::fmt::Write::write_fmt(
            &mut css,
            format_args!(".icon-{class}::before {{ content: \"\\{cp:04x}\"; }}\n"),
        );
    }

    // Append to app.css so the SPA loads exactly one stylesheet.
    // copy_styles ran first, so the file already exists.
    let app_css = dest.join("app.css");
    let mut existing =
        fs::read_to_string(&app_css).unwrap_or_else(|e| panic!("read {}: {e}", app_css.display()));
    existing.push_str("\n/* lucide-icons (generated) */\n");
    existing.push_str(&css);
    fs::write(&app_css, existing).unwrap_or_else(|e| panic!("write {}: {e}", app_css.display()));
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
