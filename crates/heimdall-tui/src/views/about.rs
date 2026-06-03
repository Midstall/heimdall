//! About view: version + enabled features pulled from `/about`.

use heimdall_i18n::t;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::app::AboutInfoTui;

pub fn render(frame: &mut Frame, area: Rect, about: Option<&AboutInfoTui>) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(t("tui.about.heading"));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines: Vec<Line> = match about {
        None => vec![Line::from(t("tui.about.loading"))],
        Some(info) => {
            let profile_label = t(format!("tui.about.profile.{}", info.build_profile).as_str());
            let enabled = info.features.enabled();
            let features_label = if enabled.is_empty() {
                t("tui.about.no_features")
            } else {
                enabled.join(", ")
            };
            vec![
                kv_line(&t("tui.about.version"), &info.version),
                kv_line(&t("tui.about.build_profile"), &profile_label),
                kv_line(&t("tui.about.features"), &features_label),
            ]
        }
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![Constraint::Length(lines.len() as u16); 1])
        .split(inner);
    let para = Paragraph::new(lines);
    frame.render_widget(para, chunks[0]);
}

fn kv_line(key: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{key:<10} "),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(value.to_string()),
    ])
}
