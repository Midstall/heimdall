//! Single-job detail rendering.

use std::collections::VecDeque;

use heimdall_daemon::Job;
use heimdall_test::Stage;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::app::JobLogEntry;

pub fn render(
    frame: &mut Frame,
    area: Rect,
    job: Option<&Job>,
    logs: Option<&VecDeque<JobLogEntry>>,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(9), Constraint::Min(0)])
        .split(area);

    render_metadata(frame, chunks[0], job);
    render_log(frame, chunks[1], logs);
}

fn render_metadata(frame: &mut Frame, area: Rect, job: Option<&Job>) {
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

fn render_log(frame: &mut Frame, area: Rect, logs: Option<&VecDeque<JobLogEntry>>) {
    let block = Block::default().borders(Borders::ALL).title("Stage Log");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let buf = match logs {
        Some(buf) if !buf.is_empty() => buf,
        _ => {
            let p = Paragraph::new(heimdall_i18n::t("web.jobs.no_logs"))
                .style(Style::default().fg(Color::DarkGray));
            frame.render_widget(p, inner);
            return;
        }
    };

    let visible_rows = inner.height as usize;
    let total = buf.len();
    let skip = total.saturating_sub(visible_rows);

    let lines: Vec<Line> = buf
        .iter()
        .skip(skip)
        .map(|e| {
            let mut spans: Vec<Span> = Vec::with_capacity(4);
            // Backend always stores UTC; user sees local time.
            let local = e.ts.with_timezone(&chrono::Local);
            spans.push(Span::styled(
                format!("{} ", local.format("%H:%M:%S%.3f")),
                Style::default().fg(Color::DarkGray),
            ));
            let level_label = heimdall_i18n::t(&format!("log.level.{}", e.level));
            spans.push(Span::styled(
                format!("{:<5} ", level_label.to_ascii_uppercase()),
                Style::default().fg(level_color(&e.level)),
            ));
            if let Some(stage) = &e.stage {
                let stage_label = stage
                    .parse::<Stage>()
                    .map(|s| heimdall_i18n::t(s.i18n_key()))
                    .unwrap_or_else(|_| stage.clone());
                spans.push(Span::styled(
                    format!("[{stage_label}] "),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ));
            }
            spans.push(Span::raw(render_message(e)));
            Line::from(spans)
        })
        .collect();

    let p = Paragraph::new(lines).wrap(Wrap { trim: false });
    frame.render_widget(p, inner);
}

/// Render the entry's message in the active locale. Prefers the
/// `i18n_key` + `i18n_args` pair when present, falling back to the
/// daemon's English `message` for entries that don't carry a key.
fn render_message(entry: &JobLogEntry) -> String {
    if let Some(key) = &entry.i18n_key {
        let args: Vec<(&str, &str)> = entry
            .i18n_args
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        return heimdall_i18n::t_args(key, &args);
    }
    entry.message.clone()
}

fn level_color(level: &str) -> Color {
    match level {
        "error" => Color::Red,
        "warn" => Color::LightYellow,
        "info" => Color::White,
        "debug" => Color::Blue,
        _ => Color::DarkGray,
    }
}

fn label(text: &str) -> Span<'_> {
    Span::styled(
        format!("{text:>10}: "),
        Style::default().add_modifier(Modifier::BOLD),
    )
}
