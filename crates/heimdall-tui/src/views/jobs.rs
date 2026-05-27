//! Jobs list rendering.

use heimdall_daemon::Job;
use ratatui::{
    Frame,
    layout::Constraint,
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Row, Table},
};

pub fn render(frame: &mut Frame, area: ratatui::layout::Rect, jobs: &[Job], focused: usize) {
    let rows: Vec<Row> = jobs
        .iter()
        .enumerate()
        .map(|(i, j)| {
            let id_short = j.id.0.to_string().chars().take(8).collect::<String>();
            let dut = j.dut.0.clone();
            let state = state_label(&j.state);
            let style = if i == focused {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            Row::new(vec![id_short, dut, state]).style(style)
        })
        .collect();
    let widths = [
        Constraint::Length(10),
        Constraint::Length(20),
        Constraint::Fill(1),
    ];
    use heimdall_i18n::t;
    let table = Table::new(rows, widths)
        .header(
            Row::new(vec![
                t("tui.jobs.col_id"),
                t("tui.jobs.col_dut"),
                t("tui.jobs.col_state"),
            ])
            .style(Style::default().add_modifier(Modifier::BOLD)),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(t("tui.jobs.title")),
        );
    frame.render_widget(table, area);
}

fn state_label(s: &heimdall_daemon::JobState) -> String {
    use heimdall_daemon::JobState;
    match s {
        JobState::Queued => "queued".into(),
        JobState::Running => "running".into(),
        JobState::Done(v) => format!("done/{}", verdict_label(v)),
        JobState::Failed(m) => format!("failed: {m}"),
        JobState::Cancelled => "cancelled".into(),
    }
}

fn verdict_label(v: &heimdall_daemon::VerdictSummary) -> &'static str {
    use heimdall_daemon::VerdictSummary;
    match v {
        VerdictSummary::Pass => "pass",
        VerdictSummary::Fail { .. } => "fail",
        VerdictSummary::Skip { .. } => "skip",
        VerdictSummary::Error { .. } => "error",
    }
}
