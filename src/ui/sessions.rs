//! 세션 화면 렌더 (PRD §8.1).
//!
//! FR-20: 좁은 터미널에서는 핵심 열만 남기고 축소한다.
//! - 폭 >= 100: 프로젝트 패널 + 시각 + 이름 + 크기 + 이유
//! - 폭 >= 72 : 크기 열을 접는다
//! - 폭 <  72 : 프로젝트 패널을 접고 프로젝트를 트리 헤더 줄로 보여준다

use crate::ops::fsutil::human_bytes;
use crate::ui::app::{App, Focus};
use crate::ui::theme::{self, *};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};

pub struct Layout2 {
    pub show_size: bool,
    /// 두 패널을 나란히 보여줄 수 있는가. 좁으면 포커스된 쪽만 전체 폭으로 그린다.
    pub side_by_side: bool,
}

pub fn layout_for(width: u16) -> Layout2 {
    Layout2 {
        show_size: width >= WIDE_ENOUGH,
        side_by_side: width >= TWO_PANE,
    }
}

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let cfg = layout_for(area.width);
    if cfg.side_by_side {
        // 프로젝트 이름 + 배지 + 상태 낱말이 들어갈 만큼은 준다.
        let pane = if cfg.show_size { 34 } else { 30 };
        let cols = Layout::horizontal([Constraint::Length(pane), Constraint::Min(24)]).split(area);
        render_projects(frame, app, cols[0]);
        render_sessions(frame, app, cols[1], &cfg);
    } else if app.focus == Focus::Projects {
        // 좁은 화면에서는 한 번에 한 패널만. `→` 로 세션 목록으로 넘어간다.
        render_projects(frame, app, area);
    } else {
        render_sessions(frame, app, area, &cfg);
    }
}

fn render_projects(frame: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == Focus::Projects;
    let visible = app.visible_projects();
    // 테두리 2칸 + 커서 기호 2칸을 뺀 나머지가 실제로 글자가 들어갈 폭이다.
    let inner = area.width.saturating_sub(4) as usize;

    let mut items: Vec<ListItem> = Vec::new();
    for &idx in &visible {
        let p = &app.result.projects[idx];
        let recommended = app.recommended_in(p);
        let selected = app.selected_in(p);

        // 상태는 색이 아니라 낱말로도 구분된다.
        let note = match p.exists {
            Some(false) => "경로없음",
            None if p.key != crate::scan::session::ORPHAN_KEY => "확인불가",
            _ => "",
        };
        let badge = if selected > 0 {
            format!("선택 {selected}")
        } else if recommended > 0 {
            format!("추천 {recommended}")
        } else {
            String::new()
        };

        // 상태 낱말("경로없음")이 잘리면 색 없이 상태를 구분할 수 없게 되므로
        // 이름을 먼저 줄이고 배지와 상태 낱말의 자리는 고정한다.
        const BADGE_W: usize = 7;
        const NOTE_W: usize = 8;
        const GUTTER: usize = 1;
        let name_w = inner.saturating_sub(BADGE_W + NOTE_W + GUTTER).max(6);
        let line = Line::from(vec![
            // 여백은 이름 밖에 둔다 — 안에 두면 이름이 꽉 찼을 때 배지와 붙는다.
            Span::raw(theme::pad(&p.short_label(), name_w)),
            Span::raw(" ".repeat(GUTTER)),
            Span::styled(
                theme::pad(&badge, BADGE_W),
                Style::default().fg(if selected > 0 { OK } else { RECOMMEND }),
            ),
            Span::styled(note.to_string(), Style::default().fg(MUTED)),
        ]);
        items.push(ListItem::new(line));
    }

    if items.is_empty() {
        items.push(ListItem::new("  프로젝트 없음"));
    }

    let title = format!(
        " Projects {}/{} ",
        (app.project_cursor + 1).min(visible.len().max(1)),
        visible.len()
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(border_style(focused));

    let mut state = ListState::default();
    state.select(Some(app.project_cursor.min(items.len().saturating_sub(1))));
    frame.render_stateful_widget(
        List::new(items)
            .block(block)
            .highlight_symbol(if focused { "▶ " } else { "  " })
            .highlight_style(if focused {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default().add_modifier(Modifier::BOLD)
            }),
        area,
        &mut state,
    );
}

fn render_sessions(frame: &mut Frame, app: &App, area: Rect, cfg: &Layout2) {
    let focused = app.focus == Focus::Sessions;
    let sessions = app.visible_sessions();
    let inner_width = area.width.saturating_sub(4) as usize;

    let mut items: Vec<ListItem> = sessions
        .iter()
        .map(|s| session_item(app, &s.id, inner_width, cfg))
        .collect();

    if items.is_empty() {
        items.push(ListItem::new(empty_message(app)));
    }

    // 어느 프로젝트를 보고 있는지 항상 제목에 밝힌다.
    let project = app
        .current_project()
        .map(|p| p.short_label())
        .unwrap_or_else(|| "—".into());
    let title = format!(" {project} — 세션 {} ", sessions.len());

    let block = Block::default()
        .borders(Borders::ALL)
        .title(theme::fit(&title, area.width.saturating_sub(14) as usize))
        .title_top(Line::from(format!(" Trash: {} ", app.trash_total_sessions())).right_aligned())
        .border_style(border_style(focused));

    let mut state = ListState::default();
    state.select(Some(app.session_cursor.min(items.len().saturating_sub(1))));
    frame.render_stateful_widget(
        List::new(items)
            .block(block)
            .highlight_symbol(if focused { "▶ " } else { "  " })
            .highlight_style(if focused {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            }),
        area,
        &mut state,
    );
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
    const GUTTER: usize = 1;
    let fixed = 4 + 2 + 9 + size.len() + GUTTER;
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
        // 이름이 열을 꽉 채워도 이유와 붙지 않게 한다.
        Span::raw(" ".repeat(GUTTER)),
        Span::styled(size, Style::default().fg(MUTED)),
        Span::styled(reason, Style::default().fg(reason_color)),
    ]))
}

fn empty_message(app: &App) -> Text<'static> {
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

    // 지금 어느 패널에 있는지에 따라 안내가 달라진다.
    // 잘린 안내는 안내가 아니다. 넓은 문구부터 실제로 들어가는지 재보고
    // 들어가는 것 중 가장 자세한 것을 쓴다. 마지막 것은 최소 폭(50)에서도 들어간다.
    let full = match app.focus {
        Focus::Projects => {
            " ↑↓ 프로젝트  → 세션 보기  Space 전체선택  A 추천전체  D 정리  T 휴지통  F 기준  ? 도움말  Q 종료"
        }
        Focus::Sessions => {
            " ↑↓ 세션  ← 프로젝트로  Space 선택  A 추천전체  D 정리  T 휴지통  F 기준  ? 도움말  Q 종료"
        }
    };
    let keys = [
        full,
        " ↑↓ 이동  ←→ 패널  Space 선택  A 추천  D 정리  T 휴지통  ? 도움말  Q 종료",
        " ↑↓←→ 이동  Space 선택  A 추천  D 정리  ? 도움말",
    ]
    .into_iter()
    .find(|k| theme::display_width(k) <= area.width as usize)
    .unwrap_or(" ? 도움말  Q 종료");
    frame.render_widget(
        Paragraph::new(theme::fit(keys, area.width as usize)).style(Style::default().fg(MUTED)),
        rows[1],
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn narrow_terminals_drop_columns_in_order() {
        assert!(layout_for(120).show_size);
        assert!(layout_for(120).side_by_side);

        let medium = layout_for(80);
        assert!(!medium.show_size, "크기 열이 먼저 접힌다");
        assert!(medium.side_by_side, "두 패널은 아직 나란히");

        let narrow = layout_for(60);
        assert!(!narrow.show_size);
        assert!(!narrow.side_by_side, "그다음 한 번에 한 패널만 보여준다");
    }
}
