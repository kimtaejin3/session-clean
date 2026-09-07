//! Confirm / result / help / recovery modals (PRD §8.2, FR-17, FR-18, FR-19).

use crate::logging;
use crate::ops::fsutil::human_bytes;
use crate::ops::manifest::CleanupMode;
use crate::ui::app::{App, DELETE_WORD};
use crate::ui::theme::*;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

/// 화면 가운데에 비율로 상자를 놓는다.
pub fn centered(area: Rect, pct_x: u16, pct_y: u16) -> Rect {
    let v = Layout::vertical([
        Constraint::Percentage((100 - pct_y) / 2),
        Constraint::Percentage(pct_y),
        Constraint::Percentage((100 - pct_y) / 2),
    ])
    .split(area);
    Layout::horizontal([
        Constraint::Percentage((100 - pct_x) / 2),
        Constraint::Percentage(pct_x),
        Constraint::Percentage((100 - pct_x) / 2),
    ])
    .split(v[1])[1]
}

fn modal<'a>(frame: &mut Frame, area: Rect, title: &'a str, lines: Vec<Line<'a>>, danger: bool) {
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {title} "))
        .border_style(Style::default().fg(if danger { DANGER } else { ACCENT }));
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

pub fn render_confirm(frame: &mut Frame, app: &App, area: Rect) {
    let box_area = centered(area, 74, 74);
    let p = &app.confirm.preview;
    let permanent = app.confirm.mode == CleanupMode::Permanent;

    let mut lines = vec![
        Line::from(format!(
            "{} · {}",
            crate::ui::theme::count(p.projects, "project"),
            crate::ui::theme::count(p.sessions, "session")
        )),
        Line::from(format!(
            "{} to move or delete · about {} freed",
            crate::ui::theme::count(p.files, "file"),
            human_bytes(p.bytes)
        )),
        Line::from(""),
    ];

    if !p.excluded.is_empty() {
        lines.push(Line::styled(
            format!(
                "{} excluded (changed since the scan, and so on)",
                crate::ui::theme::count(p.excluded.len(), "session")
            ),
            Style::default().fg(RECOMMEND),
        ));
        for (name, reason) in p.excluded.iter().take(5) {
            lines.push(Line::from(format!("  · {name} — {}", reason.label())));
        }
        if p.excluded.len() > 5 {
            lines.push(Line::from(format!("  … and {} more", p.excluded.len() - 5)));
        }
        lines.push(Line::from(""));
    }

    let trash_mark = if permanent { "  " } else { "▶ " };
    let perm_mark = if permanent { "▶ " } else { "  " };
    lines.push(Line::from(format!(
        "{trash_mark}Move to trash — you can restore it later"
    )));
    lines.push(Line::styled(
        format!("{perm_mark}Delete permanently — this cannot be undone"),
        Style::default().fg(DANGER),
    ));
    lines.push(Line::styled("  ← → to choose", Style::default().fg(MUTED)));
    lines.push(Line::from(""));

    if permanent {
        lines.push(Line::styled(
            format!("Type {DELETE_WORD} to continue: {}", app.confirm.typed),
            Style::default().fg(DANGER).add_modifier(Modifier::BOLD),
        ));
    }
    lines.push(Line::from(""));
    let ready = app.confirm.can_execute();
    lines.push(Line::styled(
        if ready {
            "Enter to run   Esc to cancel".to_string()
        } else {
            "Esc to cancel".to_string()
        },
        Style::default().fg(if ready { OK } else { MUTED }),
    ));

    modal(
        frame,
        box_area,
        if permanent {
            "Confirm permanent deletion"
        } else {
            "Confirm cleanup"
        },
        lines,
        permanent,
    );
}

pub fn render_result(frame: &mut Frame, app: &App, area: Rect) {
    let box_area = centered(area, 78, 78);
    let Some(o) = &app.outcome else { return };
    let mut lines = vec![Line::styled(
        format!(
            "{} — {} succeeded · {} skipped · {} failed",
            o.mode.map(|m| m.label()).unwrap_or("Cleanup"),
            o.succeeded.len(),
            o.skipped.len(),
            o.failed.len()
        ),
        Style::default().add_modifier(Modifier::BOLD),
    )];
    if o.bytes > 0 {
        lines.push(Line::from(format!("{} freed", human_bytes(o.bytes))));
    }
    lines.push(Line::from(""));

    section(&mut lines, "Succeeded", OK, &o.succeeded);
    if !o.skipped.is_empty() {
        lines.push(Line::styled("Skipped", Style::default().fg(RECOMMEND)));
        for (name, reason) in o.skipped.iter().take(8) {
            lines.push(Line::from(format!("  · {name} — {}", reason.label())));
        }
        lines.push(Line::from(""));
    }
    if !o.failed.is_empty() {
        lines.push(Line::styled("Failed", Style::default().fg(DANGER)));
        for (name, err) in o.failed.iter().take(8) {
            lines.push(Line::from(format!("  · {name} — {err}")));
        }
        lines.push(Line::from(""));
    }
    if o.rolled_back {
        lines.push(Line::styled(
            "The run failed, so every moved file was put back.",
            Style::default().fg(RECOMMEND),
        ));
    }
    if o.needs_attention {
        lines.push(Line::styled(
            "Recovery failed. The operation record and log were kept, and recovery will be offered next time.",
            Style::default().fg(DANGER),
        ));
    }

    lines.push(Line::styled(
        format!("Log: {}", app.paths.log_file().display()),
        Style::default().fg(MUTED),
    ));
    if app.show_log {
        for l in logging::tail(&app.paths, 12) {
            lines.push(Line::styled(format!("  {l}"), Style::default().fg(MUTED)));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::styled(
        "L show log   T trash   Esc close",
        Style::default().fg(MUTED),
    ));

    modal(frame, box_area, "Cleanup result", lines, !o.is_clean());
}

fn section(lines: &mut Vec<Line<'static>>, title: &str, color: Color, items: &[String]) {
    if items.is_empty() {
        return;
    }
    lines.push(Line::styled(title.to_string(), Style::default().fg(color)));
    for name in items.iter().take(8) {
        lines.push(Line::from(format!("  · {name}")));
    }
    if items.len() > 8 {
        lines.push(Line::from(format!("  … and {} more", items.len() - 8)));
    }
    lines.push(Line::from(""));
}

pub fn render_recovery(frame: &mut Frame, app: &App, area: Rect) {
    let box_area = centered(area, 74, 60);
    let mut lines = vec![
        Line::styled(
            "An unfinished cleanup from a previous run was found.",
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Line::from(""),
    ];
    for op in &app.pending_ops {
        lines.push(Line::from(format!(
            "  · {} — {} · {}",
            op.manifest.display_time(),
            crate::ui::theme::count(op.manifest.sessions.len(), "session"),
            crate::ui::theme::count(op.manifest.total_files(), "file")
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(
        "Recovering puts the moved files back where they were. If something already sits at the original path, it is skipped rather than overwritten.",
    ));
    lines.push(Line::from(""));
    lines.push(Line::styled(
        "R recover   Esc later",
        Style::default().fg(OK),
    ));
    modal(frame, box_area, "Recover unfinished cleanup", lines, true);
}

pub fn render_help(frame: &mut Frame, area: Rect) {
    let box_area = centered(area, 66, 82);
    let keys = [
        ("↑ / ↓", "Move within the current pane"),
        ("→", "Open the selected project's sessions"),
        ("←", "Back to the project list"),
        ("Space", "Select / deselect (whole project on the left)"),
        ("A", "Select / deselect every suggested session"),
        ("D", "Clean up the selection"),
        ("T", "Trash"),
        ("F", "Suggestion criteria"),
        ("?", "Help"),
        ("Q", "Quit"),
    ];
    let mut lines: Vec<Line> = keys
        .iter()
        .map(|(k, d)| {
            Line::from(vec![
                Span::styled(format!("  {k:<8}"), Style::default().fg(ACCENT)),
                Span::raw(*d),
            ])
        })
        .collect();
    lines.push(Line::from(""));
    lines.push(Line::styled(
        "Safety",
        Style::default().add_modifier(Modifier::BOLD),
    ));
    for note in [
        "Nothing is selected when sclean starts.",
        "Sessions whose files changed since the scan are excluded automatically.",
        "Running sessions and sessions with an unknown format are never cleaned.",
        "The trash is never emptied automatically.",
        "Restoring never overwrites a file at the original path.",
        "Your project source files are only checked for existence, never touched.",
        "No network access; everything stays on this machine.",
    ] {
        lines.push(Line::from(format!("  · {note}")));
    }
    lines.push(Line::from(""));
    lines.push(Line::styled(
        "Symbols",
        Style::default().add_modifier(Modifier::BOLD),
    ));
    lines.push(Line::from(format!(
        "  {SEL_ON} selected   {SEL_OFF} not selected   {SEL_BLOCKED} cannot clean"
    )));
    lines.push(Line::from(format!(
        "  {MARK_RECOMMENDED} suggested   {MARK_UNPARSABLE} unparseable   {MARK_RUNNING} running"
    )));
    lines.push(Line::from(""));
    lines.push(Line::styled("Esc to close", Style::default().fg(MUTED)));

    modal(frame, box_area, "Help", lines, false);
}

/// PRD §14: 터미널이 너무 작으면 아무 데이터도 건드리지 않고 안내만 한다.
pub fn render_too_small(frame: &mut Frame, area: Rect) {
    frame.render_widget(Clear, area);
    let text = vec![
        Line::from("This terminal is too small."),
        Line::from(format!("Needs at least {MIN_WIDTH} x {MIN_HEIGHT}")),
        Line::from(format!("Currently {} x {}", area.width, area.height)),
    ];
    frame.render_widget(
        Paragraph::new(text)
            .alignment(Alignment::Center)
            .style(Style::default().fg(DANGER)),
        area,
    );
}
