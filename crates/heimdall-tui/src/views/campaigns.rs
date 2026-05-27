//! Campaigns list rendering.

use heimdall_daemon::Campaign;
use ratatui::{
    Frame,
    layout::Constraint,
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Row, Table},
};

pub fn render(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    campaigns: &[Campaign],
    focused: usize,
) {
    let rows: Vec<Row> = campaigns
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let id_short = c.id.0.to_string().chars().take(8).collect::<String>();
            let dut = c.dut.0.clone();
            let template = c.template.name().to_string();
            let state = format!("{:?}", c.state).to_lowercase();
            let style = if i == focused {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            Row::new(vec![id_short, dut, template, state]).style(style)
        })
        .collect();
    let widths = [
        Constraint::Length(10),
        Constraint::Length(20),
        Constraint::Length(16),
        Constraint::Fill(1),
    ];
    use heimdall_i18n::t;
    let table = Table::new(rows, widths)
        .header(
            Row::new(vec![
                t("tui.campaigns.col_id"),
                t("tui.campaigns.col_dut"),
                t("tui.campaigns.col_template"),
                t("tui.campaigns.col_state"),
            ])
            .style(Style::default().add_modifier(Modifier::BOLD)),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(t("tui.campaigns.title")),
        );
    frame.render_widget(table, area);
}
