//! DUT registry rendering. Shows configured DUTs, transport reachability,
//! lease state, and the most recent register snapshot for the focused DUT.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap},
};

use crate::app::{App, ConnectionStatus, DutRow, DutSnapshot, DutSnapshotSource};

pub fn render(frame: &mut Frame, area: ratatui::layout::Rect, app: &App, focused: usize) {
    let duts = &app.duts;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(6), Constraint::Length(10)])
        .split(area);
    render_table(frame, chunks[0], duts, focused);

    let focused_dut = duts.get(focused);
    render_snapshot(frame, chunks[1], app, focused_dut);
}

fn render_table(frame: &mut Frame, area: ratatui::layout::Rect, duts: &[DutRow], focused: usize) {
    use heimdall_i18n::t;
    let rows: Vec<Row> = duts
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let id = Cell::from(d.id.clone());
            let kind = Cell::from(d.kind.clone());
            let serial = Cell::from(d.chip_serial.clone().unwrap_or_else(|| "-".into()));
            let jtag = Cell::from(d.jtag_driver.clone().unwrap_or_else(|| "-".into()));
            // A lease beats the transport probe: a leased DUT is by
            // definition "in use" regardless of whether the TCP port to
            // OpenOCD happens to answer right now.
            let status = if d.leased_by.is_some() {
                in_use_cell()
            } else {
                status_cell(d.connection_status)
            };
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

fn in_use_cell() -> Cell<'static> {
    Cell::from(Line::from(Span::styled(
        heimdall_i18n::t("common.status.in_use"),
        Style::default().fg(Color::Yellow),
    )))
}

fn render_snapshot(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    app: &App,
    focused: Option<&DutRow>,
) {
    use heimdall_i18n::t;
    let title = t("tui.duts.snapshot_title");
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(focused) = focused else {
        let p = Paragraph::new(t("tui.duts.snapshot_select"))
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(p, inner);
        return;
    };

    let snaps = app.snapshots_for_dut(&focused.id);
    if snaps.is_empty() {
        let p =
            Paragraph::new(t("tui.duts.snapshot_none")).style(Style::default().fg(Color::DarkGray));
        frame.render_widget(p, inner);
        return;
    }

    let mut lines: Vec<Line> = Vec::with_capacity(snaps.len() * 4);
    for snap in snaps {
        lines.extend(snapshot_lines(snap));
    }
    let p = Paragraph::new(lines).wrap(Wrap { trim: false });
    frame.render_widget(p, inner);
}

fn snapshot_lines(snap: &DutSnapshot) -> Vec<Line<'static>> {
    let source_key = match snap.source {
        DutSnapshotSource::Dut => "tui.duts.source_dut",
        DutSnapshotSource::Golden => "tui.duts.source_golden",
    };
    let local = snap.ts.with_timezone(&chrono::Local);
    let header = Line::from(vec![
        Span::styled(
            heimdall_i18n::t(source_key).to_uppercase(),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            local.format("%H:%M:%S%.3f").to_string(),
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    let mut out = vec![header];
    // Pack registers two-up to keep vertical density high in the small panel.
    // Hardware names (x{N}) get an ABI prefix so engineers don't have to
    // memorise the mapping; non-GPR keys (pc, csrs) render verbatim.
    let mut row: Vec<Span<'static>> = Vec::new();
    for (i, (name, value)) in snap.fields.iter().enumerate() {
        let label = abi_label(name);
        row.push(Span::styled(
            format!("{label:>10} "),
            Style::default().fg(Color::White),
        ));
        row.push(Span::styled(
            format!("{value:<20}"),
            Style::default().fg(Color::Yellow),
        ));
        if i % 2 == 1 {
            out.push(Line::from(std::mem::take(&mut row)));
        }
    }
    if !row.is_empty() {
        out.push(Line::from(row));
    }
    out
}

/// Render an `x{N}` State key as `a0/x10` for the register table. Falls
/// back to the bare key for non-GPR fields (pc, csrs). Mirrors the ABI
/// table in heimdall_driver::river::debug_module so the TUI doesn't drag
/// the whole driver crate in just for one lookup.
fn abi_label(name: &str) -> String {
    const ABI: [&str; 32] = [
        "zero", "ra", "sp", "gp", "tp", "t0", "t1", "t2", "fp", "s1", "a0", "a1", "a2", "a3", "a4",
        "a5", "a6", "a7", "s2", "s3", "s4", "s5", "s6", "s7", "s8", "s9", "s10", "s11", "t3", "t4",
        "t5", "t6",
    ];
    if let Some(rest) = name.strip_prefix('x') {
        if let Ok(n) = rest.parse::<usize>() {
            if n < ABI.len() {
                return format!("{}/{name}", ABI[n]);
            }
        }
    }
    name.to_string()
}
