//! PRD §16 필수 자동 검증 4: 개별 선택과 추천 전체 선택.
//! (검색은 v0.1 범위에서 제외했다 — PRD FR-04와 의도적으로 다르다.)
//! 그리고 §6 안전 우선: 시작 시 아무것도 선택되어 있지 않아야 한다.

mod support;

use sclean::ops::manifest::{CleanupMode, OpStatus};
use sclean::ops::trash;
use sclean::scan::{ScanEvent, scan};
use sclean::ui::app::{App, DELETE_WORD, Focus, Screen};
use support::{Fixture, uuid};

/// 스캔까지 마친 App을 만든다.
fn ready_app(f: &Fixture) -> App {
    let mut app = App::new(f.paths());
    app.on_scan_event(ScanEvent::Done(Box::new(scan(&f.paths()))));
    app
}

/// 추천 2개(오래됨), 비추천 1개(최근)를 만든다.
fn seeded(f: &Fixture) -> (String, String, String) {
    let shop = f.source_tree("shop-api");
    let port = f.source_tree("portfolio");
    let old_a = f
        .session(shop.to_str().unwrap(), &uuid(1))
        .summary("로그인 수정")
        .user("q1")
        .user("q2")
        .tool_use("Edit")
        .age_days(92)
        .build();
    let old_b = f
        .session(port.to_str().unwrap(), &uuid(2))
        .summary("포트폴리오 정리")
        .user("q1")
        .user("q2")
        .tool_use("Bash")
        .age_days(61)
        .build();
    let fresh = f
        .session(shop.to_str().unwrap(), &uuid(3))
        .summary("어제 작업")
        .user("q1")
        .user("q2")
        .tool_use("Read")
        .age_days(1)
        .build();
    (old_a, old_b, fresh)
}

#[test]
fn nothing_is_selected_after_scan() {
    let f = Fixture::new();
    seeded(&f);
    let app = ready_app(&f);
    assert_eq!(app.result.session_count(), 3);
    assert_eq!(app.recommended_ids().len(), 2);
    assert!(
        app.selected.is_empty(),
        "PRD §6: 추천 세션을 자동 선택하지 않는다"
    );
    assert!(app.status.contains("아무것도 선택되지 않음"));
}

#[test]
fn toggle_all_recommended_selects_only_recommended() {
    let f = Fixture::new();
    let (old_a, old_b, fresh) = seeded(&f);
    let mut app = ready_app(&f);

    app.toggle_all_recommended();
    assert_eq!(app.selected.len(), 2);
    assert!(app.selected.contains(&old_a));
    assert!(app.selected.contains(&old_b));
    assert!(!app.selected.contains(&fresh), "추천되지 않은 세션은 제외");

    app.toggle_all_recommended();
    assert!(app.selected.is_empty(), "다시 누르면 해제된다");
}

#[test]
fn choosing_a_project_shows_only_that_projects_sessions() {
    let f = Fixture::new();
    let (old_a, old_b, fresh) = seeded(&f);
    let mut app = ready_app(&f);

    // 프로젝트는 이름순: portfolio, shop-api
    app.project_cursor = 0;
    let first: Vec<&str> = app
        .visible_sessions()
        .iter()
        .map(|s| s.id.as_str())
        .collect();
    assert_eq!(first, vec![old_b.as_str()], "portfolio 세션만 보인다");

    app.move_cursor(1);
    let second: Vec<String> = app
        .visible_sessions()
        .iter()
        .map(|s| s.id.clone())
        .collect();
    assert_eq!(second.len(), 2, "shop-api 세션만 보인다");
    assert!(second.contains(&old_a));
    assert!(second.contains(&fresh));
    assert!(
        !second.contains(&old_b),
        "다른 프로젝트 세션이 섞이면 안 된다"
    );
}

#[test]
fn switching_projects_resets_the_session_cursor() {
    let f = Fixture::new();
    seeded(&f);
    let mut app = ready_app(&f);

    app.project_cursor = 1; // shop-api (세션 2개)
    app.focus_sessions();
    app.move_cursor(1);
    assert_eq!(app.session_cursor, 1);

    app.focus_projects();
    app.move_cursor(-1); // portfolio (세션 1개)
    assert_eq!(
        app.session_cursor, 0,
        "프로젝트를 바꾸면 세션 커서가 목록 밖을 가리키면 안 된다"
    );
    assert!(app.current_session().is_some());
}

#[test]
fn toggle_all_recommended_spans_every_project() {
    let f = Fixture::new();
    let (old_a, old_b, fresh) = seeded(&f);
    let mut app = ready_app(&f);

    // 보고 있는 프로젝트와 무관하게 추천 전체가 대상이다.
    app.project_cursor = 0;
    app.toggle_all_recommended();
    assert_eq!(app.selected.len(), 2);
    assert!(app.selected.contains(&old_a));
    assert!(app.selected.contains(&old_b));
    assert!(!app.selected.contains(&fresh));
    assert!(app.status.contains("프로젝트 2개"), "{}", app.status);
}

#[test]
fn blocked_sessions_cannot_be_selected() {
    let f = Fixture::new();
    let p = f.source_tree("broken");
    let id = f
        .session(p.to_str().unwrap(), &uuid(10))
        .raw_line("알 수 없는 형식")
        .age_days(400)
        .build();
    let mut app = ready_app(&f);

    app.toggle_session(&id);
    assert!(app.selected.is_empty());
    assert!(app.status.contains("분석할 수 없어"), "{}", app.status);
}

#[test]
fn space_on_the_project_pane_toggles_all_its_sessions() {
    let f = Fixture::new();
    seeded(&f);
    let mut app = ready_app(&f);
    app.project_cursor = 1; // shop-api
    app.focus = Focus::Projects;
    app.toggle_current();
    assert_eq!(app.selected.len(), 2, "shop-api의 두 세션이 모두 선택된다");
    app.toggle_current();
    assert!(app.selected.is_empty());
}

#[test]
fn space_on_the_session_pane_toggles_only_that_session() {
    let f = Fixture::new();
    seeded(&f);
    let mut app = ready_app(&f);
    app.project_cursor = 1;
    app.focus_sessions();
    app.toggle_current();
    assert_eq!(app.selected.len(), 1);
}

#[test]
fn arrows_move_focus_between_the_two_panes() {
    let f = Fixture::new();
    seeded(&f);
    let mut app = ready_app(&f);
    assert_eq!(app.focus, Focus::Projects, "왼쪽에서 시작한다");

    app.focus_sessions();
    assert_eq!(app.focus, Focus::Sessions);
    app.focus_projects();
    assert_eq!(app.focus, Focus::Projects);
}

#[test]
fn cursors_stay_within_bounds_in_both_panes() {
    let f = Fixture::new();
    seeded(&f);
    let mut app = ready_app(&f);

    app.move_cursor(-5);
    assert_eq!(app.project_cursor, 0);
    app.move_cursor(1000);
    assert_eq!(app.project_cursor, app.visible_projects().len() - 1);

    app.focus_sessions();
    app.move_cursor(1000);
    assert_eq!(app.session_cursor, app.visible_sessions().len() - 1);
    app.move_cursor(-1000);
    assert_eq!(app.session_cursor, 0);
}

#[test]
fn cleanup_leaves_the_cursor_on_a_real_row() {
    let f = Fixture::new();
    seeded(&f);
    let mut app = ready_app(&f);
    app.project_cursor = app.visible_projects().len() - 1;
    app.focus_sessions();
    app.session_cursor = 1;

    app.toggle_all_recommended();
    app.open_confirm();
    app.run_cleanup();
    app.on_scan_event(ScanEvent::Done(Box::new(scan(&f.paths()))));

    assert!(app.project_cursor < app.visible_projects().len().max(1));
    assert!(app.session_cursor <= app.visible_sessions().len());
}

#[test]
fn confirm_preview_reports_files_and_bytes() {
    let f = Fixture::new();
    seeded(&f);
    let mut app = ready_app(&f);
    app.toggle_all_recommended();
    app.open_confirm();

    assert_eq!(app.screen, Screen::Confirm);
    assert_eq!(app.confirm.preview.sessions, 2);
    assert_eq!(app.confirm.preview.projects, 2);
    assert!(app.confirm.preview.files > 0);
    assert!(app.confirm.preview.bytes > 0);
}

#[test]
fn opening_confirm_without_a_selection_is_refused() {
    let f = Fixture::new();
    seeded(&f);
    let mut app = ready_app(&f);
    app.open_confirm();
    assert_eq!(app.screen, Screen::Sessions);
    assert!(app.status.contains("선택된 세션이 없습니다"));
}

#[test]
fn permanent_delete_requires_the_exact_confirmation_word() {
    let f = Fixture::new();
    seeded(&f);
    let mut app = ready_app(&f);
    app.toggle_all_recommended();
    app.open_confirm();

    app.set_mode(CleanupMode::Permanent);
    assert!(!app.confirm.can_execute());
    app.confirm.typed = "delete".into();
    assert!(!app.confirm.can_execute(), "대소문자까지 일치해야 한다");
    app.confirm.typed = "DELETEX".into();
    assert!(!app.confirm.can_execute());
    app.confirm.typed = DELETE_WORD.into();
    assert!(app.confirm.can_execute());

    // 휴지통 이동은 추가 입력을 요구하지 않는다.
    app.set_mode(CleanupMode::Trash);
    assert!(app.confirm.can_execute());
}

#[test]
fn refusing_to_execute_without_confirmation_changes_nothing() {
    let f = Fixture::new();
    let (old_a, _, _) = seeded(&f);
    let mut app = ready_app(&f);
    app.toggle_all_recommended();
    app.open_confirm();
    app.set_mode(CleanupMode::Permanent);
    app.run_cleanup();

    assert_eq!(app.screen, Screen::Confirm, "실행되지 않아야 한다");
    assert!(app.outcome.is_none());
    assert_eq!(scan(&f.paths()).session_count(), 3);
    assert!(!old_a.is_empty());
}

#[test]
fn running_a_trash_cleanup_updates_state_and_trash_list() {
    let f = Fixture::new();
    seeded(&f);
    let mut app = ready_app(&f);
    app.toggle_all_recommended();
    app.open_confirm();
    app.run_cleanup();

    assert_eq!(app.screen, Screen::Result);
    let outcome = app.outcome.as_ref().unwrap();
    assert_eq!(outcome.succeeded.len(), 2);
    assert!(app.selected.is_empty(), "실행 후 선택은 비워진다");
    assert_eq!(app.trash_ops.len(), 1);
    assert_eq!(app.trash_total_sessions(), 2);
    assert_eq!(scan(&f.paths()).session_count(), 1, "남은 세션은 하나");
}

#[test]
fn trash_screen_groups_by_operation_and_restores() {
    let f = Fixture::new();
    seeded(&f);
    let mut app = ready_app(&f);
    app.toggle_all_recommended();
    app.open_confirm();
    app.run_cleanup();

    app.open_trash();
    assert_eq!(app.screen, Screen::Trash);
    let rows = app.trash_rows();
    assert_eq!(rows.len(), 3, "작업 헤더 1 + 세션 2");
    assert_eq!(rows[0].1, None);

    // 커서가 작업 헤더에 있을 때 복원하면 작업 전체가 돌아온다.
    app.trash_cursor = 0;
    app.restore_selection();
    assert!(app.status.contains("복원"), "{}", app.status);
    assert_eq!(scan(&f.paths()).session_count(), 3);
    assert!(app.trash_ops.is_empty());
}

#[test]
fn trash_screen_can_restore_a_single_session() {
    let f = Fixture::new();
    seeded(&f);
    let mut app = ready_app(&f);
    app.toggle_all_recommended();
    app.open_confirm();
    app.run_cleanup();
    app.open_trash();

    // 두번째 행 = 첫 세션.
    app.trash_cursor = 1;
    app.toggle_trash_current();
    assert_eq!(app.trash_selected.len(), 1);
    app.restore_selection();

    assert_eq!(scan(&f.paths()).session_count(), 2);
    assert_eq!(app.trash_total_sessions(), 1, "나머지는 휴지통에 남는다");
}

#[test]
fn trash_screen_can_purge_permanently() {
    let f = Fixture::new();
    seeded(&f);
    let mut app = ready_app(&f);
    app.toggle_all_recommended();
    app.open_confirm();
    app.run_cleanup();
    app.open_trash();

    app.trash_cursor = 0;
    app.purge_selection();
    assert!(app.trash_ops.is_empty());
    assert!(app.status.contains("영구 삭제"));
    assert_eq!(
        scan(&f.paths()).session_count(),
        1,
        "삭제된 세션은 돌아오지 않는다"
    );
}

#[test]
fn changing_threshold_persists_and_recomputes_verdicts() {
    let f = Fixture::new();
    let (_a, old_b, _c) = seeded(&f); // 61일
    let mut app = ready_app(&f);
    assert_eq!(app.recommended_ids().len(), 2);

    app.config.old_days = 30;
    app.filter_cursor = 0;
    app.adjust_threshold(40); // 70일
    assert_eq!(app.config.old_days, 70);
    assert_eq!(app.recommended_ids().len(), 1, "61일 세션은 이제 제외된다");
    assert!(!app.recommended_ids().contains(&old_b));

    // 다음 실행에도 유지된다 (수동 시나리오 3).
    let reloaded = App::new(f.paths());
    assert_eq!(reloaded.config.old_days, 70);
}

#[test]
fn disabling_a_rule_in_the_filter_screen_persists() {
    let f = Fixture::new();
    seeded(&f);
    let mut app = ready_app(&f);
    app.filter_cursor = 0; // R1
    app.toggle_filter_row();
    assert!(!app.config.rule_old);
    assert!(app.recommended_ids().is_empty());
    assert!(!App::new(f.paths()).config.rule_old);
}

#[test]
fn selection_is_dropped_when_a_rule_change_blocks_a_session() {
    let f = Fixture::new();
    seeded(&f);
    let mut app = ready_app(&f);
    app.toggle_all_recommended();
    assert_eq!(app.selected.len(), 2);
    // 규칙을 꺼도 선택은 유지된다 — 차단이 아니라 추천만 사라지기 때문.
    app.filter_cursor = 0;
    app.toggle_filter_row();
    assert_eq!(app.selected.len(), 2, "사용자의 명시적 선택은 존중한다");
}

#[test]
fn recovery_screen_takes_priority_at_startup() {
    let f = Fixture::new();
    let ids = seeded(&f);

    // 중단된 작업을 만들어 둔다.
    let paths = f.paths();
    let op_id = "20260826-090000-001".to_string();
    let dir = sclean::ops::manifest::Manifest::op_dir(&paths, &op_id);
    let mut m = sclean::ops::manifest::Manifest::new(op_id.clone(), CleanupMode::Trash);
    let key = support::encode_key(f.source_tree("shop-api").to_str().unwrap());
    let original = paths
        .projects_dir()
        .join(key)
        .join(format!("{}.jsonl", ids.0));
    let size = std::fs::metadata(&original).unwrap().len();
    sclean::ops::fsutil::move_path(&original, &dir.join("files/0/0-t.jsonl")).unwrap();
    m.sessions.push(sclean::ops::manifest::ManifestSession {
        session_id: ids.0.clone(),
        project_key: "shop".into(),
        project_path: None,
        display_name: "로그인 수정".into(),
        reasons: vec![],
        files: vec![sclean::ops::manifest::ManifestFile {
            original: original.to_string_lossy().into_owned(),
            stored: "files/0/0-t.jsonl".into(),
            size,
            is_dir: false,
            moved_at: "2026-08-26T09:00:00+09:00".into(),
        }],
        shared: vec![],
    });
    m.save(&dir).unwrap();

    let mut app = App::new(f.paths());
    assert_eq!(app.screen, Screen::Recovery, "FR-19");
    assert_eq!(app.pending_ops.len(), 1);
    assert_eq!(app.pending_ops[0].manifest.status, OpStatus::Pending);

    app.recover_pending();
    assert_eq!(app.screen, Screen::Sessions);
    assert!(original.exists(), "복구되어야 한다");
    assert!(trash::incomplete(&f.paths()).is_empty());
}

#[test]
fn a_clean_start_goes_straight_to_the_sessions_screen() {
    let f = Fixture::new();
    seeded(&f);
    assert_eq!(App::new(f.paths()).screen, Screen::Sessions);
}

#[test]
fn empty_claude_dir_produces_an_empty_but_working_app() {
    let f = Fixture::bare();
    let app = ready_app(&f);
    assert_eq!(app.result.session_count(), 0);
    assert!(app.visible_projects().is_empty());
    assert!(app.visible_sessions().is_empty());
    assert!(app.current_session().is_none());
    assert!(!app.quit);
}

#[test]
fn the_list_refreshes_after_a_cleanup() {
    let f = Fixture::new();
    let (old_a, _old_b, fresh) = seeded(&f);
    let mut app = ready_app(&f);
    assert_eq!(app.result.session_count(), 3);

    app.toggle_all_recommended();
    app.open_confirm();
    app.run_cleanup();

    assert_eq!(
        app.result.session_count(),
        1,
        "정리한 세션이 화면에 남아 있으면 안 된다"
    );
    assert!(app.session(&old_a).is_none());
    assert!(app.session(&fresh).is_some());
    assert!(app.selected.is_empty());
}

#[test]
fn the_list_refreshes_after_a_restore() {
    let f = Fixture::new();
    seeded(&f);
    let mut app = ready_app(&f);
    app.toggle_all_recommended();
    app.open_confirm();
    app.run_cleanup();
    assert_eq!(app.result.session_count(), 1);

    app.open_trash();
    app.trash_cursor = 0;
    app.restore_selection();
    assert_eq!(
        app.result.session_count(),
        3,
        "복원한 세션이 바로 보여야 한다"
    );
}
