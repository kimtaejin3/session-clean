//! PRD §15 사용성: "핵심 흐름은 키보드만으로 완료할 수 있어야 한다."
//! 실제 키 이벤트만으로 추천 확인 -> 선택 -> 휴지통 이동 -> 복원까지 끝까지 간다.

mod support;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use sclean::scan::{ScanEvent, scan};
use sclean::ui::app::{App, Screen};
use sclean::ui::handle_key;
use support::{Fixture, uuid};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

fn press(app: &mut App, code: KeyCode) {
    handle_key(app, key(code));
}

fn type_word(app: &mut App, word: &str) {
    for c in word.chars() {
        press(app, KeyCode::Char(c));
    }
}

fn ready_app(f: &Fixture) -> App {
    let mut app = App::new(f.paths());
    app.on_scan_event(ScanEvent::Done(Box::new(scan(&f.paths()))));
    app
}

fn seeded(f: &Fixture) -> String {
    let p = f.source_tree("shop-api");
    f.session(p.to_str().unwrap(), &uuid(2))
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
        .with_env()
        .age_days(92)
        .build()
}

#[test]
fn keyboard_only_flow_from_recommendation_to_trash_and_back() {
    let f = Fixture::new();
    let old = seeded(&f);
    let mut app = ready_app(&f);

    // 1. 추천 전체 선택
    press(&mut app, KeyCode::Char('a'));
    assert_eq!(app.selected.len(), 1);
    assert!(app.selected.contains(&old));

    // 2. 정리 열기
    press(&mut app, KeyCode::Char('d'));
    assert_eq!(app.screen, Screen::Confirm);
    assert_eq!(app.confirm.preview.sessions, 1);

    // 3. 휴지통 이동 실행
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.screen, Screen::Result);
    assert_eq!(app.outcome.as_ref().unwrap().succeeded.len(), 1);
    assert_eq!(scan(&f.paths()).session_count(), 1);

    // 4. 결과 -> 휴지통
    press(&mut app, KeyCode::Char('t'));
    assert_eq!(app.screen, Screen::Trash);
    assert_eq!(app.trash_total_sessions(), 1);

    // 5. 복원
    press(&mut app, KeyCode::Char('r'));
    assert_eq!(scan(&f.paths()).session_count(), 2);
    assert!(app.trash_ops.is_empty());

    // 6. 종료
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Char('q'));
    assert!(app.quit);
}

#[test]
fn permanent_delete_needs_the_typed_word_via_keyboard() {
    let f = Fixture::new();
    seeded(&f);
    let mut app = ready_app(&f);

    press(&mut app, KeyCode::Char('a'));
    press(&mut app, KeyCode::Char('d'));
    press(&mut app, KeyCode::Char('p')); // 완전 삭제 선택
    assert_eq!(
        app.confirm.mode,
        sclean::ops::manifest::CleanupMode::Permanent
    );
    press(&mut app, KeyCode::Enter);
    assert_eq!(
        app.screen,
        Screen::Confirm,
        "확인 낱말 없이는 실행되지 않는다"
    );
    assert_eq!(scan(&f.paths()).session_count(), 2);

    type_word(&mut app, "DELETE");
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.screen, Screen::Result);
    assert_eq!(scan(&f.paths()).session_count(), 1);
    assert!(
        sclean::ops::trash::list(&f.paths()).is_empty(),
        "완전 삭제는 휴지통에 남기지 않는다"
    );
}

#[test]
fn a_typo_in_the_confirmation_word_can_be_corrected_with_backspace() {
    let f = Fixture::new();
    seeded(&f);
    let mut app = ready_app(&f);
    press(&mut app, KeyCode::Char('a'));
    press(&mut app, KeyCode::Char('d'));
    press(&mut app, KeyCode::Char('p'));
    type_word(&mut app, "DELETX");
    assert_eq!(
        app.confirm.typed, "DELETX",
        "글자는 단축키로 가로채지 않는다"
    );
    assert!(!app.confirm.can_execute());
    press(&mut app, KeyCode::Backspace);
    type_word(&mut app, "E");
    assert!(app.confirm.can_execute());
}

#[test]
fn escape_cancels_the_confirmation_without_touching_anything() {
    let f = Fixture::new();
    seeded(&f);
    let mut app = ready_app(&f);
    press(&mut app, KeyCode::Char('a'));
    press(&mut app, KeyCode::Char('d'));
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Sessions);
    assert_eq!(scan(&f.paths()).session_count(), 2);
    assert_eq!(app.selected.len(), 1, "선택은 유지된다");
}

#[test]
fn search_mode_captures_letters_instead_of_shortcuts() {
    let f = Fixture::new();
    seeded(&f);
    let mut app = ready_app(&f);

    press(&mut app, KeyCode::Char('/'));
    assert!(app.searching);
    // 'd'는 정리가 아니라 검색어가 되어야 한다.
    press(&mut app, KeyCode::Char('d'));
    assert_eq!(app.screen, Screen::Sessions);
    assert_eq!(app.search, "d");

    press(&mut app, KeyCode::Esc);
    assert!(!app.searching);
    assert!(app.search.is_empty());
}

#[test]
fn help_opens_and_returns_to_the_previous_screen() {
    let f = Fixture::new();
    seeded(&f);
    let mut app = ready_app(&f);
    press(&mut app, KeyCode::Char('t'));
    press(&mut app, KeyCode::Char('?'));
    assert_eq!(app.screen, Screen::Help);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Trash, "원래 화면으로 돌아온다");
}

#[test]
fn filters_screen_adjusts_the_threshold_with_arrow_keys() {
    let f = Fixture::new();
    seeded(&f);
    let mut app = ready_app(&f);
    press(&mut app, KeyCode::Char('f'));
    assert_eq!(app.screen, Screen::Filters);

    let before = app.config.old_days;
    press(&mut app, KeyCode::Right);
    assert_eq!(app.config.old_days, before + 1);
    handle_key(
        &mut app,
        KeyEvent {
            code: KeyCode::Right,
            modifiers: KeyModifiers::SHIFT,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        },
    );
    assert_eq!(app.config.old_days, before + 8);

    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Char(' '));
    assert!(!app.config.rule_short, "두번째 줄은 R3 토글");
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Sessions);
}

#[test]
fn trash_detail_opens_with_enter_and_closes_with_escape() {
    let f = Fixture::new();
    seeded(&f);
    let mut app = ready_app(&f);
    press(&mut app, KeyCode::Char('a'));
    press(&mut app, KeyCode::Char('d'));
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Char('t'));

    press(&mut app, KeyCode::Enter);
    assert_eq!(app.screen, Screen::TrashDetail);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Trash);
}

#[test]
fn ctrl_c_quits_from_the_sessions_screen() {
    let f = Fixture::new();
    seeded(&f);
    let mut app = ready_app(&f);
    handle_key(
        &mut app,
        KeyEvent {
            code: KeyCode::Char('c'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        },
    );
    assert!(app.quit);
}

#[test]
fn right_arrow_opens_the_selected_projects_sessions() {
    let f = Fixture::new();
    seeded(&f);
    let mut app = ready_app(&f);
    assert_eq!(app.focus, sclean::ui::app::Focus::Projects);

    // 프로젝트 하나뿐이므로 그 프로젝트의 세션 2개가 보인다.
    press(&mut app, KeyCode::Right);
    assert_eq!(app.focus, sclean::ui::app::Focus::Sessions);
    assert_eq!(app.visible_sessions().len(), 2);

    press(&mut app, KeyCode::Down);
    assert_eq!(app.session_cursor, 1);
    press(&mut app, KeyCode::Down);
    assert_eq!(app.session_cursor, 1, "목록 끝에서 멈춘다");
    press(&mut app, KeyCode::Home);
    assert_eq!(app.session_cursor, 0);
    press(&mut app, KeyCode::End);
    assert_eq!(app.session_cursor, 1);

    press(&mut app, KeyCode::Left);
    assert_eq!(app.focus, sclean::ui::app::Focus::Projects);
}

#[test]
fn space_selects_the_session_under_the_cursor() {
    let f = Fixture::new();
    let old = seeded(&f);
    let mut app = ready_app(&f);
    press(&mut app, KeyCode::Right);
    // 최근 활동 순 정렬이라 92일 세션이 아래에 있다.
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Char(' '));
    assert_eq!(app.selected.len(), 1);
    assert!(app.selected.contains(&old));
}

#[test]
fn confirmation_mode_switches_with_arrows_and_never_loses_typed_letters() {
    let f = Fixture::new();
    seeded(&f);
    let mut app = ready_app(&f);
    press(&mut app, KeyCode::Char('a'));
    press(&mut app, KeyCode::Char('d'));
    assert_eq!(app.confirm.mode, sclean::ops::manifest::CleanupMode::Trash);

    press(&mut app, KeyCode::Right);
    assert_eq!(
        app.confirm.mode,
        sclean::ops::manifest::CleanupMode::Permanent
    );
    // DELETE 안의 T가 휴지통 모드로 되돌리면 안 된다.
    type_word(&mut app, "DELETE");
    assert_eq!(
        app.confirm.mode,
        sclean::ops::manifest::CleanupMode::Permanent
    );
    assert_eq!(app.confirm.typed, "DELETE");

    // 방식을 되돌리면 입력했던 낱말은 초기화된다.
    press(&mut app, KeyCode::Left);
    assert_eq!(app.confirm.mode, sclean::ops::manifest::CleanupMode::Trash);
    assert!(app.confirm.typed.is_empty());
}
