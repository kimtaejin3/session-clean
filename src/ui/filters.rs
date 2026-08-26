//! 추천 기준 화면 (PRD §8.4).

use crate::ui::app::App;
use crate::ui::modals::centered;
use crate::ui::theme::*;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let box_area = centered(area, 70, 68);
    frame.render_widget(Clear, box_area);
    let c = &app.config;

    let toggles = [
        (
            format!("오래됨 기준: {}일", c.old_days),
            c.rule_old,
            "R1  ← → 로 ±1일, Shift+← → 로 ±7일",
        ),
        (
            "짧은 세션 추천".to_string(),
            c.rule_short,
            "R3  사용자 메시지 1개 이하 + 도구 실행 없음",
        ),
        (
            "종료된 하위 에이전트 추천".to_string(),
            c.rule_subagent,
            "R4  최근 변경 중인 것은 제외",
        ),
        (
            "존재하지 않는 프로젝트 추천".to_string(),
            c.rule_missing_project,
            "R2  경로를 확인하지 못하면 적용하지 않음",
        ),
        (
            "고아 데이터 추천".to_string(),
            c.rule_orphan,
            "R5  세션 ID로 정확히 연결될 때만",
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
        "지금 기준으로 추천되는 세션: {}개",
        app.recommended_ids().len()
    )));
    lines.push(Line::from(""));
    lines.push(Line::styled(
        format!("설정 저장 위치: {}", app.paths.config_file().display()),
        Style::default().fg(MUTED),
    ));
    lines.push(Line::styled(
        "↑ ↓ 이동   Space 켜기·끄기   ← → 기준 조정   Esc 돌아가기",
        Style::default().fg(MUTED),
    ));

    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" 추천 기준 ")
                    .border_style(Style::default().fg(ACCENT)),
            )
            .wrap(Wrap { trim: false }),
        box_area,
    );
}
