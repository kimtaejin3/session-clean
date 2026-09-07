//! 휴지통 화면 (PRD §8.3).

use crate::ops::fsutil::human_bytes;
use crate::ui::app::App;
use crate::ui::modals::centered;
use crate::ui::theme::{self, *};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let rows = Layout::vertical([Constraint::Min(3), Constraint::Length(2)]).split(area);
    let mut items: Vec<ListItem> = Vec::new();

    for (i, op) in app.trash_ops.iter().enumerate() {
        let all_selected = op.manifest.sessions.iter().all(|s| {
            app.trash_selected
                .contains(&(op.manifest.op_id.clone(), s.session_id.clone()))
        });
        let check = if all_selected && !op.manifest.sessions.is_empty() {
            SEL_ON
        } else {
            SEL_OFF
        };
        items.push(ListItem::new(Line::from(vec![
            Span::raw(format!("{check} ")),
            Span::styled(
                op.manifest.display_time(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                "  {} · {} · {} · {}",
                op.manifest.mode.label(),
                theme::count(op.session_count(), "session"),
                theme::count(op.manifest.total_files(), "file"),
                human_bytes(op.bytes())
            )),
        ])));

        for s in &op.manifest.sessions {
            let key = (op.manifest.op_id.clone(), s.session_id.clone());
            let check = if app.trash_selected.contains(&key) {
                SEL_ON
            } else {
                SEL_OFF
            };
            items.push(ListItem::new(Line::from(vec![
                Span::raw(format!("   {check} ")),
                Span::raw(fit(&s.display_name, 34)),
                Span::styled(
                    format!("  {}", human_bytes(s.bytes())),
                    Style::default().fg(MUTED),
                ),
            ])));
        }
        let _ = i;
    }

    if items.is_empty() {
        items.push(ListItem::new("  The trash is empty."));
    }

    let total = human_bytes(crate::ops::trash::total_bytes(&app.trash_ops));
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(
            " Trash — {} · {total} ",
            theme::count(app.trash_total_sessions(), "session")
        ))
        .border_style(Style::default().fg(ACCENT));

    let mut state = ListState::default();
    state.select(Some(app.trash_cursor.min(items.len().saturating_sub(1))));
    frame.render_stateful_widget(
        List::new(items)
            .block(block)
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
        rows[0],
        &mut state,
    );

    let footer = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(rows[1]);
    frame.render_widget(
        Paragraph::new(format!(" {}", app.status)).style(Style::default().fg(ACCENT)),
        footer[0],
    );
    frame.render_widget(
        Paragraph::new(" Space select  R restore  X delete permanently  Enter details  Esc back")
            .style(Style::default().fg(MUTED)),
        footer[1],
    );
}

/// `Enter`: 포함된 프로젝트, 세션과 원래 경로 확인 (PRD §8.3).
pub fn render_detail(frame: &mut Frame, app: &App, area: Rect) {
    let box_area = centered(area, 84, 76);
    frame.render_widget(Clear, box_area);

    let rows = app.trash_rows();
    let Some(&(oi, si)) = rows.get(app.trash_cursor) else {
        return;
    };
    let op = &app.trash_ops[oi];
    let sessions: Vec<&crate::ops::manifest::ManifestSession> = match si {
        Some(j) => vec![&op.manifest.sessions[j]],
        None => op.manifest.sessions.iter().collect(),
    };

    let mut lines = vec![
        Line::styled(
            format!(
                "{} · {}",
                op.manifest.display_time(),
                op.manifest.mode.label()
            ),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Line::from(""),
    ];
    for s in sessions {
        lines.push(Line::styled(
            format!("{} ({})", s.display_name, s.session_id),
            Style::default().fg(ACCENT),
        ));
        if let Some(p) = &s.project_path {
            lines.push(Line::from(format!("  Project: {p}")));
        }
        if !s.reasons.is_empty() {
            lines.push(Line::styled(
                format!("  Reason: {}", s.reasons.join(" · ")),
                Style::default().fg(RECOMMEND),
            ));
        }
        for f in &s.files {
            lines.push(Line::styled(
                format!("  · {} ({})", f.original, human_bytes(f.size)),
                Style::default().fg(MUTED),
            ));
        }
        if !s.shared.is_empty() {
            lines.push(Line::styled(
                format!(
                    "  · {} of shared record",
                    theme::count(s.shared.len(), "line")
                ),
                Style::default().fg(MUTED),
            ));
        }
        lines.push(Line::from(""));
    }
    lines.push(Line::styled("Esc to close", Style::default().fg(MUTED)));

    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Trash details ")
                    .border_style(Style::default().fg(ACCENT)),
            )
            .wrap(Wrap { trim: false }),
        box_area,
    );
}
