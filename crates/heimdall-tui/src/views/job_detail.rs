//! Single-job detail rendering.

use heimdall_daemon::Job;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

pub fn render(frame: &mut Frame, area: Rect, job: Option<&Job>) {
    let lines: Vec<Line> = match job {
        None => vec![Line::from("Job not loaded yet")],
        Some(j) => {
            let id = j.id.0.to_string();
            let mut v = vec![
                Line::from(vec![label("Id"), Span::raw(id)]),
                Line::from(vec![label("DUT"), Span::raw(j.dut.0.clone())]),
                Line::from(vec![label("Kind"), Span::raw(format!("{:?}", j.kind))]),
                Line::from(vec![label("State"), Span::raw(format!("{:?}", j.state))]),
                Line::from(vec![label("Created"), Span::raw(j.created_at.to_rfc3339())]),
                Line::from(vec![label("Updated"), Span::raw(j.updated_at.to_rfc3339())]),
            ];
            if let Some(c) = &j.campaign {
                v.push(Line::from(vec![
                    label("Campaign"),
                    Span::raw(c.0.to_string()),
                ]));
            }
            v
        }
    };
    let p = Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title("Job Detail"));
    frame.render_widget(p, area);
}

fn label(text: &str) -> Span<'_> {
    Span::styled(
        format!("{text:>10}: "),
        Style::default().add_modifier(Modifier::BOLD),
    )
}
