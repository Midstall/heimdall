//! Emits the TypeScript bindings for heimdall-disasm's wire types into
//! `crates/heimdall-web/frontend/src/bindings/`. Run with
//! `cargo test -p heimdall-disasm --features bindings`. The emitted
//! files are checked into git; rerun this test whenever you change
//! a type that the frontend consumes.

#![cfg(feature = "bindings")]

use heimdall_disasm::{DisasmLine, DisasmListing, Isa};
use ts_rs::{Config, TS};

#[test]
fn export_disasm_bindings() {
    let cfg = Config::new()
        // u64 fields (DisasmLine::addr, ...) come across the wire as
        // plain JSON numbers (serde encodes them that way for values
        // fitting in safe-integer range), and the frontend treats them
        // as `number`. Default `bigint` would force annoying casts.
        .with_large_int("number")
        // ES-module imports in the bundle are served as `.js`. ts-rs
        // adds the dot separator itself, so we pass the bare suffix
        // (`"js"`, not `".js"`) and the resulting import reads
        // `./Foo.js`, matching what the browser fetches from /assets/.
        .with_import_extension(Some("js"));
    // export_all() recursively exports referenced types too, so calling
    // it on the top-level DisasmListing pulls Isa + DisasmLine along.
    // Each #[ts(export_to = ...)] attribute on those types decides
    // where the .ts file lands.
    DisasmListing::export_all(&cfg).expect("export DisasmListing");
    // Defensive: also export Isa standalone in case DisasmListing
    // changes to no longer reference it (the bindings/ checked-in
    // file would otherwise rot).
    Isa::export(&cfg).expect("export Isa");
    DisasmLine::export(&cfg).expect("export DisasmLine");
}
