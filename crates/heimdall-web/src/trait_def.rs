use async_trait::async_trait;
use heimdall_i18n::Locale;
use thiserror::Error;

use crate::ssr::IndexData;

/// Concrete error surface for [`WebContext`] implementations. Lets the
/// router log a properly-typed error and the implementor pass through
/// its own error taxonomy via the `Backend` variant (the daemon
/// forwards its `DaemonError` here).
#[derive(Debug, Error)]
pub enum WebContextError {
    /// The backing storage (job store, dut registry, etc.) returned
    /// an error while building the SSR index data. Carries the
    /// originating error's Display rendering so the router can
    /// surface it verbatim in its 500 response without depending on
    /// a downstream error type.
    #[error("ssr backend error: {0}")]
    Backend(String),
}

impl WebContextError {
    /// Wrap any `Display`-able backend error into a `Backend` variant.
    /// The daemon's `WebContext` impl uses this to flatten its
    /// `DaemonError` chain into the trait's error type without
    /// pulling daemon-specific types into heimdall-web.
    pub fn backend<E: std::fmt::Display>(err: E) -> Self {
        Self::Backend(err.to_string())
    }
}

/// Provider of the data the SSR index template needs. Implemented by
/// the daemon (against its `AppState`); the heimdall-web crate stays
/// independent of the storage stack so it can also be used by a thin
/// preview-only binary (e.g. `cargo run --example serve_static`).
#[async_trait]
pub trait WebContext: Send + Sync + 'static {
    async fn build_index_data(&self, locale: Locale) -> Result<IndexData, WebContextError>;
}
