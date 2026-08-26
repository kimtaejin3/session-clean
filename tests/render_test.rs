//! 렌더링 스모크 테스트.
//!
//! 실제 화면 버퍼를 만들어 내용을 확인한다. 패닉 없이 그려지는지,
//! 그리고 PRD가 요구한 정보(추천 이유, 선택 상태, 키 안내)가 실제로
//! 화면에 나타나는지 본다. 좁은 터미널 축소(FR-20)도 여기서 확인한다.

mod support;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use sclean::scan::{ScanEvent, scan};
use sclean::ui::app::{App, Screen};
use sclean::ui::{filters, modals, sessions, theme, trash};
use support::{Fixture, uuid};

fn ready_app(f: &Fixture) -> App {
    let mut app = App::new(f.paths());
    app.on_scan_event(ScanEvent::Done(Box::new(scan(&f.paths()))));
    app
}

fn seeded(f: &Fixture) -> String {
    let p = f.source_tree("shop-api");
    f.session(p.to_str().unwrap(), &uuid(9))
        .summary("어제 작업")
        .user("q1")
        .user("q2")
        .tool_use("Read")
        .age_days(1)
        .build();
    f.session(p.to_str().unwrap(), &uuid(1))
        .summary("로그인 수정")
        .user("q1")
        .user("q2")
        .tool_use("Edit")
        .age_days(92)
        .build()
}

/// 화면 전체를 한 줄씩 문자열로 뽑는다.
fn draw(app: &App, w: u16, h: u16, screen: Screen) -> String {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal
        .draw(|frame| {
            let area = frame.area();
            if area.width < theme::MIN_WIDTH || area.height < theme::MIN_HEIGHT {
                modals::render_too_small(frame, area);
                return;
            }
            use ratatui::layout::{Constraint, Layout};
            let rows = Layout::vertical([Constraint::Min(3), Constraint::Length(2)]).split(area);
            match screen {
                Screen::Trash => trash::render(frame, app, area),
                Screen::TrashDetail => {
                    trash::render(frame, app, area);
                    trash::render_detail(frame, app, area);
                }
                _ => {
                    sessions::render(frame, app, rows[0]);
                    sessions::render_footer(frame, app, rows[1]);
                    match screen {
                        Screen::Confirm => modals::render_confirm(frame, app, area),
                        Screen::Result => modals::render_result(frame, app, area),
                        Screen::Filters => filters::render(frame, app, area),
                        Screen::Help => modals::render_help(frame, area),
                        Screen::Recovery => modals::render_recovery(frame, app, area),
                        _ => {}
                    }
                }
            }
        })
        .unwrap();
    let buf = terminal.backend().buffer().clone();
    // 전각 문자는 두 칸을 차지하고 두번째 칸은 자리표시자다.
    // 그대로 이으면 "로 그 인"이 되므로 이어지는 칸을 건너뛴다.
    (0..buf.area.height)
        .map(|y| {
            let mut line = String::new();
            let mut x = 0u16;
            while x < buf.area.width {
                let symbol = buf[(x, y)].symbol();
                line.push_str(symbol);
                let w = symbol.chars().map(theme::display_width_of).sum::<usize>();
                x += w.max(1) as u16;
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn sessions_screen_shows_names_reasons_and_keys() {
    let f = Fixture::new();
    seeded(&f);
    let app = ready_app(&f);
    let out = draw(&app, 110, 24, Screen::Sessions);

    assert!(out.contains("Projects"), "{out}");
    assert!(out.contains("Sessions"));
    assert!(out.contains("shop-api"));
    assert!(out.contains("로그인 수정"), "세션 이름이 보여야 한다");
    assert!(out.contains("마지막 활동 후"), "추천 이유가 보여야 한다");
    assert!(out.contains("추천 1"), "프로젝트별 추천 수");
    assert!(out.contains("[ ]"), "선택 상태를 기호로 표시");
    assert!(out.contains("★"), "추천을 기호로 표시");
    assert!(out.contains("Trash: 0"));
    assert!(out.contains("Space 선택"), "핵심 조작은 항상 하단에");
    assert!(out.contains("아무것도 선택되지 않음"));
}

#[test]
fn selecting_a_session_changes_its_visible_mark() {
    let f = Fixture::new();
    seeded(&f);
    let mut app = ready_app(&f);
    app.toggle_all_recommended();
    let out = draw(&app, 110, 24, Screen::Sessions);
    assert!(out.contains("[x]"), "선택은 색이 아니라 기호로 드러난다");
    assert!(out.contains("선택 1개"));
}

#[test]
fn narrow_terminal_drops_columns_and_keeps_working() {
    let f = Fixture::new();
    seeded(&f);
    let app = ready_app(&f);

    let wide = draw(&app, 110, 24, Screen::Sessions);
    assert!(
        wide.contains("KB") || wide.contains(" B"),
        "넓으면 크기 열이 있다"
    );

    let medium = draw(&app, 80, 20, Screen::Sessions);
    assert!(medium.contains("로그인 수정"));
    assert!(medium.contains("Projects"), "80칸에서는 프로젝트 패널 유지");

    let narrow = draw(&app, 60, 16, Screen::Sessions);
    assert!(
        !narrow.contains("Projects"),
        "좁으면 프로젝트 패널을 접는다"
    );
    assert!(narrow.contains("shop-api"), "대신 헤더 줄로 보여준다");
    assert!(narrow.contains("로그인 수정"));
}

#[test]
fn too_small_terminal_shows_guidance_only() {
    let f = Fixture::new();
    seeded(&f);
    let app = ready_app(&f);
    let out = draw(&app, 40, 8, Screen::Sessions);
    assert!(out.contains("너무 작습니다"));
    assert!(out.contains("50"));
    assert!(!out.contains("로그인 수정"), "데이터를 그리지 않는다");
}

#[test]
fn confirm_modal_shows_counts_size_and_both_modes() {
    let f = Fixture::new();
    seeded(&f);
    let mut app = ready_app(&f);
    app.toggle_all_recommended();
    app.open_confirm();
    let out = draw(&app, 110, 26, Screen::Confirm);

    assert!(out.contains("정리 확인"));
    assert!(out.contains("세션 1개"));
    assert!(out.contains("예상 정리 용량"));
    assert!(out.contains("휴지통 이동"));
    assert!(out.contains("완전 삭제"));
    assert!(out.contains("Enter 실행"));
}

#[test]
fn permanent_mode_demands_the_delete_word_on_screen() {
    let f = Fixture::new();
    seeded(&f);
    let mut app = ready_app(&f);
    app.toggle_all_recommended();
    app.open_confirm();
    app.set_mode(sclean::ops::manifest::CleanupMode::Permanent);
    let out = draw(&app, 110, 26, Screen::Confirm);
    assert!(out.contains("완전 삭제 확인"));
    assert!(out.contains("DELETE"));
    assert!(
        !out.contains("Enter 실행"),
        "입력 전에는 실행을 안내하지 않는다"
    );
}

#[test]
fn result_screen_separates_success_skipped_and_failed() {
    let f = Fixture::new();
    seeded(&f);
    let mut app = ready_app(&f);
    app.toggle_all_recommended();
    app.open_confirm();
    app.run_cleanup();
    let out = draw(&app, 110, 26, Screen::Result);

    assert!(out.contains("정리 결과"));
    assert!(out.contains("성공 1"));
    assert!(out.contains("로그:"), "FR-18: 로그 경로를 보여준다");
}

#[test]
fn trash_screen_lists_operations_and_sessions() {
    let f = Fixture::new();
    seeded(&f);
    let mut app = ready_app(&f);
    app.toggle_all_recommended();
    app.open_confirm();
    app.run_cleanup();
    app.open_trash();

    let out = draw(&app, 110, 24, Screen::Trash);
    assert!(out.contains("Trash"));
    assert!(out.contains("휴지통 이동"));
    assert!(out.contains("로그인 수정"));
    assert!(out.contains("R 복원"));
    assert!(out.contains("X 영구 삭제"));

    let detail = draw(&app, 110, 24, Screen::TrashDetail);
    assert!(detail.contains("휴지통 상세"));
    assert!(
        detail.contains("마지막 활동 후"),
        "정리한 이유를 보관·표시한다"
    );
}

#[test]
fn filters_screen_shows_every_rule_and_the_threshold() {
    let f = Fixture::new();
    seeded(&f);
    let app = ready_app(&f);
    let out = draw(&app, 110, 26, Screen::Filters);

    assert!(out.contains("추천 기준"));
    assert!(out.contains("오래됨 기준: 30일"));
    assert!(out.contains("짧은 세션 추천"));
    assert!(out.contains("종료된 하위 에이전트 추천"));
    assert!(out.contains("존재하지 않는 프로젝트 추천"));
    assert!(out.contains("고아 데이터 추천"));
    assert!(out.contains("config.json"), "저장 위치를 밝힌다");
}

#[test]
fn help_screen_lists_keys_and_safety_policy() {
    let f = Fixture::new();
    seeded(&f);
    let app = ready_app(&f);
    let out = draw(&app, 110, 30, Screen::Help);
    assert!(out.contains("도움말"));
    assert!(out.contains("추천 항목 전체"));
    assert!(out.contains("안전 정책"));
    assert!(out.contains("덮어쓰지 않습니다"));
    assert!(out.contains("네트워크"));
}

#[test]
fn empty_state_explains_where_it_looked() {
    let f = Fixture::bare();
    let app = ready_app(&f);
    let out = draw(&app, 110, 20, Screen::Sessions);
    assert!(out.contains("Claude Code 세션을 찾지 못했"), "{out}");
    assert!(out.contains("확인한 경로"));
}

#[test]
fn unparsable_session_is_shown_but_marked_blocked() {
    let f = Fixture::new();
    let p = f.source_tree("broken");
    f.session(p.to_str().unwrap(), &uuid(30))
        .raw_line("알 수 없는 형식")
        .age_days(200)
        .build();
    let app = ready_app(&f);
    let out = draw(&app, 110, 20, Screen::Sessions);

    assert!(out.contains("분석 불가"), "안전하게 표시한다 (FR-15)");
    assert!(out.contains("[-]"), "정리 불가는 기호로 구분된다");
    assert!(!out.contains("[x]"));
}

#[test]
fn scan_progress_is_visible_while_scanning() {
    let f = Fixture::new();
    seeded(&f);
    let mut app = App::new(f.paths());
    app.on_scan_event(ScanEvent::Progress {
        done: 1203,
        total: 2000,
    });
    let out = draw(&app, 110, 20, Screen::Sessions);
    assert!(out.contains("스캔 중 1203 / 2000"), "{out}");
}

#[test]
fn recovery_modal_explains_what_will_happen() {
    let f = Fixture::new();
    seeded(&f);
    let mut app = ready_app(&f);
    app.pending_ops = vec![];
    // 실제 중단 작업을 만들어 화면을 채운다.
    app.toggle_all_recommended();
    app.open_confirm();
    app.run_cleanup();
    app.pending_ops = sclean::ops::trash::list(&f.paths());

    let out = draw(&app, 110, 24, Screen::Recovery);
    assert!(out.contains("중단된 작업 복구"));
    assert!(out.contains("덮어쓰지 않고"));
    assert!(out.contains("R 복구"));
}
