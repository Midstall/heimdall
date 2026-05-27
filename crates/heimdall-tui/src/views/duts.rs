//! DUT registry rendering. Shows configured DUTs, transport reachability,
//! and lease state.

use ratatui::{
    Frame,
    layout::Constraint,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Row, Table},
};

use crate::app::{ConnectionStatus, DutRow};

pub fn render(frame: &mut Frame, area: ratatui::layout::Rect, duts: &[DutRow], focused: usize) {
    use heimdall_i18n::t;
    let rows: Vec<Row> = duts
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let id = Cell::from(d.id.clone());
            let kind = Cell::from(d.kind.clone());
            let serial = Cell::from(d.chip_serial.clone().unwrap_or_else(|| "-".into()));
            let jtag = Cell::from(d.jtag_driver.clone().unwrap_or_else(|| "-".into()));
            let status = status_cell(d.connection_status);
            let lease = Cell::from(
                d.leased_by
                    .as_ref()
                    .map(|h| {
                        let short = h.chars().take(8).collect::<String>();
                        heimdall_i18n::t!("tui.duts.leased_by", holder = short)
                    })
                    .unwrap_or_else(|| t("common.status.idle")),
            );
            let row_style = if i == focused {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            Row::new(vec![id, kind, serial, jtag, status, lease]).style(row_style)
        })
        .collect();

    let widths = [
        Constraint::Length(20),
        Constraint::Length(18),
        Constraint::Length(14),
        Constraint::Length(10),
        Constraint::Length(14),
        Constraint::Fill(1),
    ];
    let table = Table::new(rows, widths)
        .header(
            Row::new(vec![
                t("tui.duts.col_id"),
                t("tui.duts.col_kind"),
                t("tui.duts.col_serial"),
                t("tui.duts.col_jtag"),
                t("tui.duts.col_status"),
                t("tui.duts.col_lease"),
            ])
            .style(Style::default().add_modifier(Modifier::BOLD)),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(t("tui.duts.title")),
        );
    frame.render_widget(table, area);
}

/// Render a `ConnectionStatus` with color-coding:
/// connected = green, disconnected = red, unknown/idle = dim.
fn status_cell(status: ConnectionStatus) -> Cell<'static> {
    let (key, color) = match status {
        ConnectionStatus::Connected => ("common.status.connected", Color::Green),
        ConnectionStatus::Disconnected => ("common.status.disconnected", Color::Red),
        ConnectionStatus::Unknown => ("common.status.idle", Color::DarkGray),
    };
    Cell::from(Line::from(Span::styled(
        heimdall_i18n::t(key),
        Style::default().fg(color),
    )))
}
