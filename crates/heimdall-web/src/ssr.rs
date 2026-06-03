//! Plain-data structs the SSR template renders. Kept as dumb POD with
//! public fields so the daemon (and future implementors) can construct
//! them directly from their own types without going through a builder
//! layer.

use askama::Template;
use heimdall_i18n::Locale;

/// One row in the SSR'd jobs table on the index page.
#[derive(Debug, Clone)]
pub struct JobRow {
    pub id_short: String,
    pub dut: String,
    pub kind: String,
    pub state_class: String,
    pub state_label: String,
    pub created_at: String,
}

/// One row in the SSR'd campaigns table on the index page.
#[derive(Debug, Clone)]
pub struct CampaignRow {
    pub id_short: String,
    pub dut: String,
    pub template: String,
    pub state: String,
    pub chip_serial: String,
}

/// One card in the SSR'd DUTs grid on the index page.
#[derive(Debug, Clone)]
pub struct DutCardRow {
    pub id: String,
    pub kind: String,
    pub chip_serial: String,
    pub jtag_driver: String,
    /// CSS class for the status badge (`"connected"` / `"disconnected"` /
    /// `"idle"` / `"in-use"`). Stays in English so the colour rules
    /// don't need to change per locale.
    pub status_class: &'static str,
    /// Localised status label that ships back to the browser.
    pub status_label: String,
    pub has_netlist: bool,
}

/// Pre-rendered data bundle the [`crate::WebContext`] returns to the
/// index handler. The handler stitches this into the [`IndexTemplate`]
/// along with the locale.
#[derive(Debug, Clone)]
pub struct IndexData {
    pub jobs: Vec<JobRow>,
    pub campaigns: Vec<CampaignRow>,
    pub duts: Vec<DutCardRow>,
}

#[derive(Template)]
#[template(path = "index.html")]
pub(crate) struct IndexTemplate {
    pub(crate) locale: String,
    pub(crate) jobs: Vec<JobRow>,
    pub(crate) campaigns: Vec<CampaignRow>,
    pub(crate) duts: Vec<DutCardRow>,
    /// Locale used for `self.trans()` lookups. Held as enum so we don't
    /// have to round-trip through a string per call.
    #[allow(dead_code)]
    pub(crate) locale_kind: Locale,
}

impl IndexTemplate {
    /// Translation helper called from the template.
    pub fn trans(&self, key: &str) -> String {
        heimdall_i18n::t_in(self.locale_kind, key)
    }
}
