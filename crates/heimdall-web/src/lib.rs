//! Heimdall web UI. Owns the SSR templates, the static-asset embed,
//! and the Axum router that serves them. Generic over a [`WebContext`]
//! trait so the crate doesn't need to depend on the daemon's storage
//! stack; the daemon implements the trait against its [`AppState`].

pub mod locale;
pub mod router;
pub mod ssr;
pub mod trait_def;

pub use router::router;
pub use ssr::{CampaignRow, DutCardRow, IndexData, JobRow};
pub use trait_def::{WebContext, WebContextError};
