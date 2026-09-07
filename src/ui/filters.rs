//! 추천 기준 화면 (PRD §8.4).

use crate::ui::app::App;
use crate::ui::modals::centered;
use crate::ui::theme::*;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let box_area = centered(area, 88, 92);
    frame.render_widget(Clear, box_area);
    let c = &app.config;

    let toggles = [
        (
            format!("Older than {} days", c.old_days),
            c.rule_old,
            "R1  ← → adjusts by 1 day, Shift+← → by 7",
        ),
        (
            "Suggest short sessions".to_string(),
            c.rule_short,
            "R3  at most one user message and no tool calls",
        ),
        (
            "Suggest finished subagents".to_string(),
            c.rule_subagent,
            "R4  skips ones that changed recently",
        ),
        (
            "Suggest sessions of missing projects".to_string(),
            c.rule_missing_project,
            "R2  not applied when the path cannot be confirmed",
        ),
        (
            "Suggest orphaned data".to_string(),
            c.rule_orphan,
            "R5  only when it maps to an exact session id",
        ),
    ];

    let mut lines = Vec::new();
    for (i, (label, on, note)) in toggles.iter().enumerate() {
        let cursor = if app.filter_cursor == i { "▶" } else { " " };
        let mark = if *on { SEL_ON } else { SEL_OFF };
        lines.push(Line::from(vec![Span::styled(
            format!("{cursor} {mark} {label}"),
            Style::default().fg(if app.filter_cursor == i {
                ACCENT
            } else {
                Color::Reset
            }),
        )]));
        lines.push(Line::styled(
            format!("      {note}"),
            Style::default().fg(MUTED),
        ));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(format!(
        "Suggested with these criteria: {}",
        app.recommended_ids().len()
    )));
    lines.push(Line::from(""));
    lines.push(Line::styled(
        format!("Saved in {}", app.paths.config_file().display()),
        Style::default().fg(MUTED),
    ));
    lines.push(Line::styled(
        "↑ ↓ move   Space toggle   ← → adjust   Esc back",
        Style::default().fg(MUTED),
    ));

    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Suggestion criteria ")
                    .border_style(Style::default().fg(ACCENT)),
            )
            .wrap(Wrap { trim: false }),
        box_area,
    );
}
