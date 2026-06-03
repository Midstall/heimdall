//! Daemon-side facade over [`heimdall_api`]: re-exports the wire
//! types verbatim so existing `heimdall_daemon::{JobKind, JobState,
//! ...}` imports keep working. Cross-crate conversions live in the
//! crate that owns the source-side type:
//!
//! - `From<&heimdall_core::Verdict>` lives in `heimdall-api`
//!   (Verdict is from heimdall-core, which heimdall-api already
//!   depends on).
//! - `From<heimdall_test::SnapshotSource>` lives in `heimdall-test`
//!   (orphan rule needs the local `SnapshotSource`).

pub use heimdall_api::*;

/// Build an [`AboutInfo`] reflecting the running daemon binary.
/// `cfg!(feature = ...)` only sees the daemon's own feature flags,
/// not heimdall-api's, so the snapshot lives here rather than as a
/// `Default`-style constructor on the wire type.
pub fn current_about_info() -> AboutInfo {
    AboutInfo {
        version: heimdall_core::VERSION.to_string(),
        build_profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
        .to_string(),
        features: AboutFeatures {
            sqlite: cfg!(feature = "sqlite"),
            aegis: cfg!(feature = "aegis"),
            river: cfg!(feature = "river"),
            fuzzer: cfg!(feature = "fuzzer"),
            cranelift: cfg!(feature = "cranelift"),
        },
    }
}
