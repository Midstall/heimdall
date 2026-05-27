//! App state: which view is active, cached state from the daemon, focus.

use heimdall_daemon::{Campaign, Job};
use serde::Deserialize;

/// Live state of the TUI -> daemon connection. Reflected in the status bar
/// and used by the reconnect loop to decide whether to surface "trying to
/// reconnect" UX.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ConnectionState {
    /// Initial state: no successful connect attempt yet.
    Connecting,
    /// At least one WS handshake completed, HTTP polls succeeding.
    Connected,
    /// WS closed or HTTP poll failed. Reconnect task is retrying.
    #[default]
    Disconnected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum View {
    Jobs,
    JobDetail { id: String },
    Campaigns,
    Duts,
}

/// Reachability of a DUT's transport as reported by the daemon's `/duts`
/// probe. `Unknown` is the default when the daemon can't determine the
/// state (mock transport, optional feature disabled).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ConnectionStatus {
    Connected,
    Disconnected,
    #[default]
    Unknown,
}

/// Minimal projection of `heimdall_daemon::DutRecord` for the TUI. The daemon
/// struct itself isn't `Deserialize`, and the TUI only needs a flat view of
/// the registered DUTs + which are currently leased + live reachability.
#[derive(Debug, Clone, Deserialize)]
pub struct DutRow {
    pub id: String,
    pub kind: String,
    #[serde(default)]
    pub chip_serial: Option<String>,
    /// Free-text summary of the JTAG transport driver (e.g. "mock", "ftdi",
    /// "openocd"). Filled in from the embedded `jtag.driver` field.
    #[serde(default)]
    pub jtag_driver: Option<String>,
    /// Job id holding the lease on this DUT, if any.
    #[serde(default)]
    pub leased_by: Option<String>,
    /// Live reachability of the configured transport. `Unknown` if the
    /// daemon couldn't probe (mock transport, ftdi feature off, etc).
    #[serde(default)]
    pub connection_status: ConnectionStatus,
}

#[derive(Debug)]
pub struct App {
    pub view: View,
    pub jobs: Vec<Job>,
    pub campaigns: Vec<Campaign>,
    pub duts: Vec<DutRow>,
    pub focused_index: usize,
    pub status: String,
    pub connection: ConnectionState,
    pub should_quit: bool,
}

impl App {
    pub fn new() -> Self {
        Self {
            view: View::Jobs,
            jobs: Vec::new(),
            campaigns: Vec::new(),
            duts: Vec::new(),
            focused_index: 0,
            status: heimdall_i18n::t("common.status.connecting"),
            connection: ConnectionState::Connecting,
            should_quit: false,
        }
    }

    /// Mark the connection as lost. Called from the event loop when the WS
    /// drops or an HTTP poll fails.
    pub fn mark_disconnected(&mut self, reason: impl Into<String>) {
        self.connection = ConnectionState::Disconnected;
        let reason = reason.into();
        self.status = if reason.is_empty() {
            heimdall_i18n::t("tui.disconnected_short")
        } else {
            heimdall_i18n::t!("tui.disconnected_reason", reason = reason)
        };
    }

    /// Mark the connection as healthy. Called when the WS reconnects.
    pub fn mark_connected(&mut self, daemon_url: &str) {
        self.connection = ConnectionState::Connected;
        self.status = heimdall_i18n::t!("tui.connected_to", url = daemon_url);
    }

    pub fn focused_job(&self) -> Option<&Job> {
        self.jobs.get(self.focused_index)
    }

    pub fn focused_campaign(&self) -> Option<&Campaign> {
        self.campaigns.get(self.focused_index)
    }

    pub fn focused_dut(&self) -> Option<&DutRow> {
        self.duts.get(self.focused_index)
    }

    pub fn navigate_down(&mut self) {
        let cap = self.focus_cap();
        if cap == 0 {
            return;
        }
        if self.focused_index + 1 < cap {
            self.focused_index += 1;
        }
    }

    pub fn navigate_up(&mut self) {
        if self.focused_index > 0 {
            self.focused_index -= 1;
        }
    }

    pub fn switch_view(&mut self, view: View) {
        self.view = view;
        self.focused_index = 0;
    }

    fn focus_cap(&self) -> usize {
        match self.view {
            View::Jobs => self.jobs.len(),
            View::Campaigns => self.campaigns.len(),
            View::Duts => self.duts.len(),
            View::JobDetail { .. } => 0,
        }
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_clamps_within_jobs() {
        let mut app = App::new();
        // Synthesize three jobs via JSON for brevity.
        app.jobs = serde_json::from_str(r#"
            [
                {"id":"00000000-0000-0000-0000-000000000001","dut":"d1","dut_kind":"river-rc1-nano","kind":{"kind":"mock-hello"},"campaign":null,"state":{"state":"queued"},"created_at":"2026-05-25T00:00:00Z","updated_at":"2026-05-25T00:00:00Z"},
                {"id":"00000000-0000-0000-0000-000000000002","dut":"d2","dut_kind":"river-rc1-nano","kind":{"kind":"mock-hello"},"campaign":null,"state":{"state":"queued"},"created_at":"2026-05-25T00:00:00Z","updated_at":"2026-05-25T00:00:00Z"},
                {"id":"00000000-0000-0000-0000-000000000003","dut":"d3","dut_kind":"river-rc1-nano","kind":{"kind":"mock-hello"},"campaign":null,"state":{"state":"queued"},"created_at":"2026-05-25T00:00:00Z","updated_at":"2026-05-25T00:00:00Z"}
            ]
        "#).unwrap();
        for _ in 0..5 {
            app.navigate_down();
        }
        assert_eq!(app.focused_index, 2);
        for _ in 0..5 {
            app.navigate_up();
        }
        assert_eq!(app.focused_index, 0);
    }

    #[test]
    fn switch_view_resets_focus() {
        let mut app = App::new();
        app.focused_index = 5;
        app.switch_view(View::Campaigns);
        assert_eq!(app.focused_index, 0);
    }

    #[test]
    fn duts_view_navigation_clamps_within_list() {
        let mut app = App::new();
        app.switch_view(View::Duts);
        app.duts = vec![
            DutRow {
                id: "d1".into(),
                kind: "river-rc1-nano".into(),
                chip_serial: None,
                jtag_driver: Some("mock".into()),
                leased_by: None,
                connection_status: ConnectionStatus::Unknown,
            },
            DutRow {
                id: "d2".into(),
                kind: "aegis-luna-1".into(),
                chip_serial: Some("SN-2".into()),
                jtag_driver: Some("ftdi".into()),
                leased_by: Some("00000000-0000-0000-0000-000000000001".into()),
                connection_status: ConnectionStatus::Connected,
            },
        ];
        for _ in 0..5 {
            app.navigate_down();
        }
        assert_eq!(app.focused_index, 1);
        assert_eq!(app.focused_dut().unwrap().id, "d2");
    }

    #[test]
    fn focused_dut_is_none_when_list_empty() {
        let mut app = App::new();
        app.switch_view(View::Duts);
        assert!(app.focused_dut().is_none());
    }
}
