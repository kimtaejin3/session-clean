//! 정리 확인 / 결과 / 도움말 / 복구 모달 (PRD §8.2, FR-17, FR-18, FR-19).

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
        Paragraph::new(lines).block(block).wrap(Wrap { trim: false }),
        area,
    );
}

pub fn render_confirm(frame: &mut Frame, app: &App, area: Rect) {
    let box_area = centered(area, 74, 74);
    let p = &app.confirm.preview;
    let permanent = app.confirm.mode == CleanupMode::Permanent;

    let mut lines = vec![
        Line::from(format!(
            "프로젝트 {}개 · 세션 {}개",
            p.projects, p.sessions
        )),
        Line::from(format!(
            "이동/삭제할 파일 {}개 · 예상 정리 용량 {}",
            p.files,
            human_bytes(p.bytes)
        )),
        Line::from(""),
    ];

    if !p.excluded.is_empty() {
        lines.push(Line::styled(
            format!("제외된 세션 {}개 (스캔 이후 변경 등)", p.excluded.len()),
            Style::default().fg(RECOMMEND),
        ));
        for (name, reason) in p.excluded.iter().take(5) {
            lines.push(Line::from(format!("  · {name} — {}", reason.label())));
        }
        if p.excluded.len() > 5 {
            lines.push(Line::from(format!("  … 외 {}개", p.excluded.len() - 5)));
        }
        lines.push(Line::from(""));
    }

    let trash_mark = if permanent { " " } else { "▶" };
    let perm_mark = if permanent { "▶" } else { " " };
    lines.push(Line::from(format!("{trash_mark} [T] 휴지통 이동 — 나중에 복원할 수 있습니다")));
    lines.push(Line::styled(
        format!("{perm_mark} [P] 완전 삭제 — 되돌릴 수 없습니다"),
        Style::default().fg(DANGER),
    ));
    lines.push(Line::from(""));

    if permanent {
        lines.push(Line::styled(
            format!("계속하려면 {DELETE_WORD} 를 입력하세요: {}", app.confirm.typed),
            Style::default().fg(DANGER).add_modifier(Modifier::BOLD),
        ));
    }
    lines.push(Line::from(""));
    let ready = app.confirm.can_execute();
    lines.push(Line::styled(
        if ready {
            "Enter 실행   Esc 취소".to_string()
        } else {
            "Esc 취소".to_string()
        },
        Style::default().fg(if ready { OK } else { MUTED }),
    ));

    modal(
        frame,
        box_area,
        if permanent { "완전 삭제 확인" } else { "정리 확인" },
        lines,
        permanent,
    );
}

pub fn render_result(frame: &mut Frame, app: &App, area: Rect) {
    let box_area = centered(area, 78, 78);
    let Some(o) = &app.outcome else { return };
    let mut lines = vec![Line::styled(
        format!(
            "{} — 성공 {} · 제외 {} · 실패 {}",
            o.mode.map(|m| m.label()).unwrap_or("정리"),
            o.succeeded.len(),
            o.skipped.len(),
            o.failed.len()
        ),
        Style::default().add_modifier(Modifier::BOLD),
    )];
    if o.bytes > 0 {
        lines.push(Line::from(format!("정리한 용량 {}", human_bytes(o.bytes))));
    }
    lines.push(Line::from(""));

    section(&mut lines, "성공", OK, &o.succeeded);
    if !o.skipped.is_empty() {
        lines.push(Line::styled("제외", Style::default().fg(RECOMMEND)));
        for (name, reason) in o.skipped.iter().take(8) {
            lines.push(Line::from(format!("  · {name} — {}", reason.label())));
        }
        lines.push(Line::from(""));
    }
    if !o.failed.is_empty() {
        lines.push(Line::styled("실패", Style::default().fg(DANGER)));
        for (name, err) in o.failed.iter().take(8) {
            lines.push(Line::from(format!("  · {name} — {err}")));
        }
        lines.push(Line::from(""));
    }
    if o.rolled_back {
        lines.push(Line::styled(
            "실패해서 이동한 파일을 모두 원래 자리로 되돌렸습니다.",
            Style::default().fg(RECOMMEND),
        ));
    }
    if o.needs_attention {
        lines.push(Line::styled(
            "복구에 실패했습니다. 작업 기록과 로그를 보존했고 다음 실행에서 복구를 제안합니다.",
            Style::default().fg(DANGER),
        ));
    }

    lines.push(Line::styled(
        format!("로그: {}", app.paths.log_file().display()),
        Style::default().fg(MUTED),
    ));
    if app.show_log {
        for l in logging::tail(&app.paths, 12) {
            lines.push(Line::styled(
                format!("  {l}"),
                Style::default().fg(MUTED),
            ));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::styled(
        "L 로그 보기   T 휴지통   Esc 닫기",
        Style::default().fg(MUTED),
    ));

    modal(frame, box_area, "정리 결과", lines, !o.is_clean());
}

fn section(lines: &mut Vec<Line<'static>>, title: &str, color: Color, items: &[String]) {
    if items.is_empty() {
        return;
    }
    lines.push(Line::styled(
        title.to_string(),
        Style::default().fg(color),
    ));
    for name in items.iter().take(8) {
        lines.push(Line::from(format!("  · {name}")));
    }
    if items.len() > 8 {
        lines.push(Line::from(format!("  … 외 {}개", items.len() - 8)));
    }
    lines.push(Line::from(""));
}

pub fn render_recovery(frame: &mut Frame, app: &App, area: Rect) {
    let box_area = centered(area, 74, 60);
    let mut lines = vec![
        Line::styled(
            "이전 실행에서 끝나지 않은 정리 작업을 찾았습니다.",
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Line::from(""),
    ];
    for op in &app.pending_ops {
        lines.push(Line::from(format!(
            "  · {} — 세션 {}개 · 파일 {}개",
            op.manifest.display_time(),
            op.manifest.sessions.len(),
            op.manifest.total_files()
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(
        "복구하면 옮겨진 파일을 원래 자리로 되돌립니다. 같은 경로에 파일이 이미 있으면 덮어쓰지 않고 건너뜁니다.",
    ));
    lines.push(Line::from(""));
    lines.push(Line::styled(
        "R 복구   Esc 나중에",
        Style::default().fg(OK),
    ));
    modal(frame, box_area, "중단된 작업 복구", lines, true);
}

pub fn render_help(frame: &mut Frame, area: Rect) {
    let box_area = centered(area, 66, 82);
    let keys = [
        ("↑ / ↓", "항목 이동"),
        ("← / →", "프로젝트 접기·펼치기 / 패널 이동"),
        ("Space", "현재 항목 선택·해제"),
        ("A", "현재 필터에서 추천 항목 전체 선택·해제"),
        ("/", "프로젝트·세션 검색"),
        ("D", "선택 항목 정리"),
        ("T", "휴지통 화면"),
        ("F", "추천 기준 화면"),
        ("?", "도움말"),
        ("Q", "종료"),
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
    lines.push(Line::styled("안전 정책", Style::default().add_modifier(Modifier::BOLD)));
    for note in [
        "시작할 때 아무것도 선택되어 있지 않습니다.",
        "실행 직전 파일이 바뀐 세션은 자동으로 제외합니다.",
        "실행 중인 세션과 형식을 알 수 없는 세션은 정리하지 않습니다.",
        "휴지통은 자동으로 비워지지 않습니다.",
        "복원할 때 같은 경로의 파일을 덮어쓰지 않습니다.",
        "프로젝트 소스 파일은 존재 확인 외에 건드리지 않습니다.",
        "네트워크를 쓰지 않고 모든 데이터는 이 Mac에만 남습니다.",
    ] {
        lines.push(Line::from(format!("  · {note}")));
    }
    lines.push(Line::from(""));
    lines.push(Line::styled("기호", Style::default().add_modifier(Modifier::BOLD)));
    lines.push(Line::from(format!(
        "  {SEL_ON} 선택   {SEL_OFF} 미선택   {SEL_BLOCKED} 정리 불가"
    )));
    lines.push(Line::from(format!(
        "  {MARK_RECOMMENDED} 추천   {MARK_UNPARSABLE} 분석 불가   {MARK_RUNNING} 실행 중"
    )));
    lines.push(Line::from(""));
    lines.push(Line::styled("Esc 닫기", Style::default().fg(MUTED)));

    modal(frame, box_area, "도움말", lines, false);
}

/// PRD §14: 터미널이 너무 작으면 아무 데이터도 건드리지 않고 안내만 한다.
pub fn render_too_small(frame: &mut Frame, area: Rect) {
    frame.render_widget(Clear, area);
    let text = vec![
        Line::from("터미널이 너무 작습니다."),
        Line::from(format!("최소 {MIN_WIDTH} x {MIN_HEIGHT} 필요")),
        Line::from(format!("현재 {} x {}", area.width, area.height)),
    ];
    frame.render_widget(
        Paragraph::new(text)
            .alignment(Alignment::Center)
            .style(Style::default().fg(DANGER)),
        area,
    );
}
