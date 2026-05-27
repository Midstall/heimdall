//! Top-level render: status bar / body / help bar layout.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::app::{App, ConnectionState, View};

pub fn render(frame: &mut Frame, app: &App) {
    use heimdall_i18n::t;
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);

    // Status bar (top): <brand>  |  <view-label>: <name>  |  [conn-pill] <status>
    let (conn_label_key, conn_color) = match app.connection {
        ConnectionState::Connected => ("tui.conn.connected", Color::Green),
        ConnectionState::Connecting => ("tui.conn.connecting", Color::Yellow),
        ConnectionState::Disconnected => ("tui.conn.disconnected", Color::Red),
    };
    let brand = t("tui.brand");
    let view_lbl = t("tui.view_label");
    let conn_label = t(conn_label_key);
    let status_line = Line::from(vec![
        Span::raw(" "),
        Span::raw(brand),
        Span::raw("  |  "),
        Span::raw(view_lbl),
        Span::raw(": "),
        Span::raw(view_label(&app.view)),
        Span::raw("  |  "),
        Span::styled(
            format!("[{conn_label}]"),
            Style::default().fg(conn_color).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::raw(app.status.clone()),
    ]);
    let status =
        Paragraph::new(status_line).style(Style::default().bg(Color::DarkGray).fg(Color::White));
    frame.render_widget(status, chunks[0]);

    // Body
    match &app.view {
        View::Jobs => crate::views::jobs::render(frame, chunks[1], &app.jobs, app.focused_index),
        View::Campaigns => {
            crate::views::campaigns::render(frame, chunks[1], &app.campaigns, app.focused_index)
        }
        View::Duts => crate::views::duts::render(frame, chunks[1], &app.duts, app.focused_index),
        View::JobDetail { id } => {
            let job = app.jobs.iter().find(|j| j.id.0.to_string() == *id);
            crate::views::job_detail::render(frame, chunks[1], job);
        }
    }

    // Help bar (bottom)
    let help = format!(
        " {} | {} | {} | {} | {} | {} | {} | {} ",
        t("tui.help.quit"),
        t("tui.help.jobs"),
        t("tui.help.campaigns"),
        t("tui.help.duts"),
        t("tui.help.move"),
        t("tui.help.open"),
        t("tui.help.back"),
        t("tui.help.refresh"),
    );
    let help_para = Paragraph::new(Line::from(help))
        .style(Style::default().bg(Color::DarkGray).fg(Color::White));
    frame.render_widget(help_para, chunks[2]);
}

fn view_label(v: &View) -> String {
    use heimdall_i18n::t;
    match v {
        View::Jobs => t("tui.view.jobs"),
        View::Campaigns => t("tui.view.campaigns"),
        View::Duts => t("tui.view.duts"),
        View::JobDetail { .. } => t("tui.view.job_detail"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::sync::{Mutex, OnceLock};

    /// Locale-sensitive tests share a global through `heimdall_i18n`. The
    /// lock serializes them so parallel test execution doesn't observe a
    /// half-flipped locale. Every UI render test acquires this lock and
    /// re-asserts the default English locale on entry.
    fn locale_test_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let m = LOCK.get_or_init(|| Mutex::new(()));
        let guard = m.lock().unwrap_or_else(|e| e.into_inner());
        heimdall_i18n::set_locale(heimdall_i18n::Locale::En);
        guard
    }

    #[test]
    fn renders_to_test_backend_without_panic() {
        let _g = locale_test_lock();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let app = App::new();
        terminal.draw(|f| render(f, &app)).unwrap();
        let buf = terminal.backend().buffer();
        let dumped = buf_to_string(buf);
        assert!(dumped.contains("heimdall tui"));
        assert!(dumped.contains("jobs"));
    }

    #[test]
    fn help_bar_mentions_duts_shortcut() {
        let _g = locale_test_lock();
        let backend = TestBackend::new(120, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        let app = App::new();
        terminal.draw(|f| render(f, &app)).unwrap();
        let dumped = buf_to_string(terminal.backend().buffer());
        assert!(
            dumped.contains("3 duts"),
            "help bar should list `3 duts`:\n{dumped}"
        );
    }

    #[test]
    fn duts_view_renders_populated_table() {
        use crate::app::{ConnectionStatus, DutRow, View};
        let _g = locale_test_lock();
        let backend = TestBackend::new(120, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        app.switch_view(View::Duts);
        app.duts = vec![DutRow {
            id: "river-1".into(),
            kind: "river-rc1-nano".into(),
            chip_serial: Some("SN-001".into()),
            jtag_driver: Some("ftdi".into()),
            leased_by: None,
            connection_status: ConnectionStatus::Connected,
        }];
        terminal.draw(|f| render(f, &app)).unwrap();
        let dumped = buf_to_string(terminal.backend().buffer());
        assert!(
            dumped.contains("DUTs"),
            "expected DUTs panel title:\n{dumped}"
        );
        assert!(dumped.contains("river-1"));
        assert!(dumped.contains("ftdi"));
        assert!(dumped.contains("connected"));
        assert!(
            dumped.contains("idle"),
            "lease column should still read 'idle'"
        );
    }

    #[test]
    fn duts_view_marks_leased_dut() {
        use crate::app::{ConnectionStatus, DutRow, View};
        let _g = locale_test_lock();
        let backend = TestBackend::new(120, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        app.switch_view(View::Duts);
        app.duts = vec![DutRow {
            id: "river-1".into(),
            kind: "river-rc1-nano".into(),
            chip_serial: None,
            jtag_driver: Some("mock".into()),
            leased_by: Some("abcdef0123456789".into()),
            connection_status: ConnectionStatus::Unknown,
        }];
        terminal.draw(|f| render(f, &app)).unwrap();
        let dumped = buf_to_string(terminal.backend().buffer());
        assert!(
            dumped.contains("leased (abcdef01)"),
            "expected leased-by short id:\n{dumped}"
        );
    }

    #[test]
    fn duts_view_shows_disconnected_status() {
        use crate::app::{ConnectionStatus, DutRow, View};
        let _g = locale_test_lock();
        let backend = TestBackend::new(120, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        app.switch_view(View::Duts);
        app.duts = vec![DutRow {
            id: "ftdi-1".into(),
            kind: "aegis-luna1".into(),
            chip_serial: None,
            jtag_driver: Some("ftdi".into()),
            leased_by: None,
            connection_status: ConnectionStatus::Disconnected,
        }];
        terminal.draw(|f| render(f, &app)).unwrap();
        let dumped = buf_to_string(terminal.backend().buffer());
        assert!(
            dumped.contains("disconnected"),
            "expected disconnected status:\n{dumped}"
        );
    }

    #[test]
    fn japanese_locale_translates_view_label_and_titles() {
        use crate::app::{ConnectionStatus, DutRow, View};
        // Hold the locale lock for the full test so concurrent EN tests
        // wait until we restore. Force Japanese inside the critical section.
        let _g = locale_test_lock();
        let prev = heimdall_i18n::set_locale(heimdall_i18n::Locale::Ja);
        let backend = TestBackend::new(120, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        app.switch_view(View::Duts);
        app.duts = vec![DutRow {
            id: "river-1".into(),
            kind: "river-rc1-nano".into(),
            chip_serial: None,
            jtag_driver: Some("ftdi".into()),
            leased_by: None,
            connection_status: ConnectionStatus::Connected,
        }];
        terminal.draw(|f| render(f, &app)).unwrap();
        let dumped = buf_to_string(terminal.backend().buffer());
        heimdall_i18n::set_locale(prev);

        // The TestBackend pads wide (CJK) characters with a blank cell, so
        // "一覧" renders as "一 覧" in the dump. Strip all whitespace before
        // matching so the assertions stay legible.
        let no_ws: String = dumped.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            no_ws.contains("DUT一覧"),
            "expected Japanese DUT title:\n{dumped}"
        );
        assert!(
            no_ws.contains("接続済み"),
            "expected Japanese 'connected' label:\n{dumped}"
        );
        assert!(
            no_ws.contains("終了"),
            "expected Japanese 'quit' in help bar:\n{dumped}"
        );
    }

    #[test]
    fn duts_view_unknown_status_renders_as_idle() {
        use crate::app::{ConnectionStatus, DutRow, View};
        let _g = locale_test_lock();
        let backend = TestBackend::new(120, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        app.switch_view(View::Duts);
        app.duts = vec![DutRow {
            id: "mock-1".into(),
            kind: "river-rc1-nano".into(),
            chip_serial: None,
            jtag_driver: Some("mock".into()),
            leased_by: None,
            connection_status: ConnectionStatus::Unknown,
        }];
        terminal.draw(|f| render(f, &app)).unwrap();
        let dumped = buf_to_string(terminal.backend().buffer());
        // Status cell renders "idle" for Unknown. Lease cell also reads
        // "idle" when no lease is held. Both should be present.
        let count = dumped.matches("idle").count();
        assert!(
            count >= 2,
            "expected both status and lease to show idle:\n{dumped}"
        );
    }

    fn buf_to_string(buf: &ratatui::buffer::Buffer) -> String {
        let area = buf.area();
        let mut out = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                let cell = &buf[(x, y)];
                out.push_str(cell.symbol());
            }
            out.push('\n');
        }
        out
    }
}
