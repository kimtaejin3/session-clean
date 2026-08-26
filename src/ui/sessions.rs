//! 세션 화면 렌더 (PRD §8.1).
//!
//! FR-20: 좁은 터미널에서는 핵심 열만 남기고 축소한다.
//! - 폭 >= 100: 프로젝트 패널 + 시각 + 이름 + 크기 + 이유
//! - 폭 >= 72 : 크기 열을 접는다
//! - 폭 <  72 : 프로젝트 패널을 접고 프로젝트를 트리 헤더 줄로 보여준다

use crate::ops::fsutil::human_bytes;
use crate::ui::app::{App, Focus, Row};
use crate::ui::theme::{self, *};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};

pub struct Layout2 {
    pub show_size: bool,
    pub show_project_pane: bool,
}

pub fn layout_for(width: u16) -> Layout2 {
    Layout2 {
        show_size: width >= WIDE_ENOUGH,
        show_project_pane: width >= TWO_PANE,
    }
}

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let cfg = layout_for(area.width);
    if cfg.show_project_pane {
        let cols = Layout::horizontal([Constraint::Length(26), Constraint::Min(20)]).split(area);
        render_projects(frame, app, cols[0]);
        render_sessions(frame, app, cols[1], &cfg);
    } else {
        render_sessions(frame, app, area, &cfg);
    }
}

fn render_projects(frame: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == Focus::Projects;
    let items: Vec<ListItem> = app
        .result
        .projects
        .iter()
        .map(|p| {
            let open = if app.collapsed.contains(&p.key) {
                COLLAPSED
            } else {
                EXPANDED
            };
            let recommended = p
                .sessions
                .iter()
                .filter(|s| app.verdict(&s.id).is_some_and(|v| v.recommended()))
                .count();
            // 존재하지 않는 프로젝트는 색이 아니라 낱말로도 표시한다.
            let mark = match p.exists {
                Some(false) => " 경로없음",
                None if p.key != crate::scan::session::ORPHAN_KEY => " 확인불가",
                _ => "",
            };
            let name = theme::fit(&p.short_label(), 14);
            let line = format!(
                "{open} {} {}{}",
                theme::pad(&name, 14),
                if recommended > 0 {
                    format!("추천 {recommended}")
                } else {
                    "     ".into()
                },
                mark
            );
            ListItem::new(line)
        })
        .collect();

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Projects ")
        .border_style(border_style(focused));
    frame.render_widget(List::new(items).block(block), area);
}

fn render_sessions(frame: &mut Frame, app: &App, area: Rect, cfg: &Layout2) {
    let inner_width = area.width.saturating_sub(2) as usize;
    let mut items: Vec<ListItem> = Vec::new();

    for row in &app.rows {
        match row {
            Row::Project { key } => {
                if cfg.show_project_pane {
                    continue;
                }
                // 좁은 화면: 프로젝트를 헤더 줄로 끼워 넣는다.
                let p = app.result.projects.iter().find(|p| &p.key == key);
                let label = p.map(|p| p.short_label()).unwrap_or_default();
                items.push(ListItem::new(Line::from(vec![Span::styled(
                    format!("── {label} "),
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                )])));
            }
            Row::Session { id, .. } => {
                items.push(session_item(app, id, inner_width, cfg));
            }
        }
    }

    if items.is_empty() {
        items.push(ListItem::new(empty_message(app)));
    }

    let title = if app.search.is_empty() {
        " Sessions ".to_string()
    } else {
        format!(" Sessions — 검색: {} ", app.search)
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_top(Line::from(format!(" Trash: {} ", app.trash_total_sessions())).right_aligned())
        .border_style(border_style(app.focus == Focus::Sessions));

    let mut state = ListState::default();
    // 프로젝트 행이 빠질 수 있으므로 커서를 항목 인덱스로 환산한다.
    state.select(Some(visible_index(app, cfg)));
    frame.render_stateful_widget(
        List::new(items)
            .block(block)
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
        area,
        &mut state,
    );
}

fn visible_index(app: &App, cfg: &Layout2) -> usize {
    if !cfg.show_project_pane {
        return app.cursor;
    }
    app.rows
        .iter()
        .take(app.cursor + 1)
        .filter(|r| r.session_id().is_some())
        .count()
        .saturating_sub(1)
}

fn session_item<'a>(app: &'a App, id: &str, width: usize, cfg: &Layout2) -> ListItem<'a> {
    let Some(session) = app.session(id) else {
        return ListItem::new("");
    };
    let verdict = app.verdict(id);
    let recommended = verdict.is_some_and(|v| v.recommended());
    let cleanable = verdict.is_some_and(|v| v.cleanable());

    let check = if app.selected.contains(id) {
        SEL_ON
    } else if cleanable {
        SEL_OFF
    } else {
        SEL_BLOCKED
    };
    let mark = if !cleanable && session.analysis.is_unreadable() {
        MARK_UNPARSABLE
    } else if app.live.contains(id) {
        MARK_RUNNING
    } else if recommended {
        MARK_RECOMMENDED
    } else {
        MARK_PLAIN
    };

    let when = theme::pad(&relative_time(app.now(), session.last_active_secs), 8);
    let size = if cfg.show_size {
        theme::pad(&human_bytes(session.size_bytes), 9)
    } else {
        String::new()
    };

    // 이름과 이유가 나눠 쓸 수 있는 폭.
    let fixed = 4 + 2 + 8 + 1 + size.len() + 1;
    let rest = width.saturating_sub(fixed).max(20);
    let name_w = (rest * 2 / 5).max(12);
    let reason_w = rest.saturating_sub(name_w);
    let name = theme::pad(&session.display_name, name_w);
    let reason = theme::fit(&verdict.map(|v| v.label()).unwrap_or_default(), reason_w);

    let reason_color = if !cleanable {
        DANGER
    } else if recommended {
        RECOMMEND
    } else {
        MUTED
    };

    ListItem::new(Line::from(vec![
        Span::raw(format!("{check} ")),
        Span::styled(
            format!("{mark} "),
            Style::default().fg(if recommended { RECOMMEND } else { MUTED }),
        ),
        Span::raw(format!("{when} ")),
        Span::raw(name),
        Span::styled(size, Style::default().fg(MUTED)),
        Span::styled(reason, Style::default().fg(reason_color)),
    ]))
}

fn empty_message(app: &App) -> Text<'static> {
    if !app.search.is_empty() {
        return Text::from(format!("  '{}' 와(과) 맞는 항목이 없습니다", app.search));
    }
    if !app.paths.claude_dir_exists() {
        return Text::from(vec![
            Line::from("  Claude Code 세션을 찾지 못했습니다."),
            Line::from(format!("  확인한 경로: {}", app.paths.claude_dir.display())),
        ]);
    }
    Text::from("  정리할 세션이 없습니다.")
}

fn border_style(focused: bool) -> Style {
    if focused {
        Style::default().fg(ACCENT)
    } else {
        Style::default().fg(MUTED)
    }
}

/// 하단 상태줄 + 키 안내 (PRD §8.5: 핵심 조작은 항상 표시).
pub fn render_footer(frame: &mut Frame, app: &App, area: Rect) {
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(area);

    let status = match &app.scan_state {
        crate::ui::app::ScanState::Scanning { done, total } if *total > 0 => {
            format!(" 스캔 중 {done} / {total}")
        }
        crate::ui::app::ScanState::Scanning { .. } => " 스캔 중…".to_string(),
        crate::ui::app::ScanState::Ready => {
            let n = app.selected.len();
            if n > 0 {
                format!(
                    " {} · 선택 {n}개 ({})",
                    app.status,
                    human_bytes(app.total_selected_bytes())
                )
            } else {
                format!(" {}", app.status)
            }
        }
    };
    frame.render_widget(
        Paragraph::new(status).style(Style::default().fg(ACCENT)),
        rows[0],
    );

    let keys = if app.searching {
        " 입력: 검색  Enter 확정  Esc 취소"
    } else {
        " Space 선택  A 추천 전체  D 정리  T 휴지통  F 기준  / 검색  ? 도움말  Q 종료"
    };
    frame.render_widget(
        Paragraph::new(keys).style(Style::default().fg(MUTED)),
        rows[1],
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn narrow_terminals_drop_columns_in_order() {
        assert!(layout_for(120).show_size);
        assert!(layout_for(120).show_project_pane);

        let medium = layout_for(80);
        assert!(!medium.show_size, "크기 열이 먼저 접힌다");
        assert!(medium.show_project_pane);

        let narrow = layout_for(60);
        assert!(!narrow.show_size);
        assert!(!narrow.show_project_pane, "그다음 프로젝트 패널이 접힌다");
    }
}
