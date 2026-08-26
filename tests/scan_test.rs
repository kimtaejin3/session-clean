//! PRD §16 필수 자동 검증 1, 3: 탐색·그룹화, 손상된 JSONL의 안전한 처리.

mod support;

use sclean::scan::session::{ORPHAN_KEY, SessionKind};
use sclean::scan::{jsonl::Analysis, scan};
use support::{Fixture, uuid};

#[test]
fn groups_sessions_by_project() {
    let f = Fixture::new();
    let a = f.source_tree("shop-api");
    let b = f.source_tree("portfolio");
    f.session(a.to_str().unwrap(), &uuid(1))
        .summary("로그인 수정")
        .user("첫 질문")
        .build();
    f.session(a.to_str().unwrap(), &uuid(2))
        .user("두번째 세션")
        .build();
    f.session(b.to_str().unwrap(), &uuid(3))
        .user("포트폴리오")
        .build();

    let result = scan(&f.paths());
    assert_eq!(result.projects.len(), 2);
    assert_eq!(result.session_count(), 3);

    let shop = result
        .projects
        .iter()
        .find(|p| p.short_label() == "shop-api")
        .expect("shop-api 그룹");
    assert_eq!(shop.sessions.len(), 2);
    assert_eq!(shop.exists, Some(true));
    assert_eq!(shop.path.as_deref(), Some(a.as_path()));
    assert!(
        shop.sessions
            .iter()
            .any(|s| s.display_name == "로그인 수정"),
        "summary를 표시 이름으로 쓴다"
    );
}

#[test]
fn missing_projects_dir_yields_empty_result_without_error() {
    let f = Fixture::bare();
    let result = scan(&f.paths());
    assert_eq!(result.session_count(), 0);
    assert!(result.errors.is_empty(), "없는 경로는 오류가 아니다");
}

#[test]
fn corrupted_session_is_listed_but_marked_unanalyzable() {
    let f = Fixture::new();
    let p = f.source_tree("broken");
    let id = f
        .session(p.to_str().unwrap(), &uuid(7))
        .raw_line("이건 JSON이 아니다")
        .raw_line("<html>")
        .build();

    let result = scan(&f.paths());
    let s = result
        .find(&support::encode_key(p.to_str().unwrap()), &id)
        .unwrap();
    assert!(matches!(s.analysis, Analysis::Unreadable(_)));
    assert!(
        s.display_name.starts_with("분석 불가"),
        "안전하게 표시해야 한다: {}",
        s.display_name
    );
}

#[test]
fn partially_broken_session_keeps_readable_information() {
    let f = Fixture::new();
    let p = f.source_tree("partial");
    let id = f
        .session(p.to_str().unwrap(), &uuid(8))
        .user("읽히는 질문")
        .raw_line("{ 깨진 줄")
        .build();

    let result = scan(&f.paths());
    let s = result
        .find(&support::encode_key(p.to_str().unwrap()), &id)
        .unwrap();
    assert!(matches!(s.analysis, Analysis::Partial(_)));
    assert_eq!(s.display_name, "읽히는 질문");
}

#[test]
fn missing_project_path_is_detected_from_cwd() {
    let f = Fixture::new();
    // 프로젝트 소스 트리를 만들지 않는다 = 삭제된 프로젝트.
    let ghost = f.dir.path().join("work/deleted-project");
    f.session(ghost.to_str().unwrap(), &uuid(9))
        .user("x")
        .build();

    let result = scan(&f.paths());
    let s = result.sessions().next().unwrap();
    assert_eq!(s.project_exists, Some(false));
}

#[test]
fn project_without_cwd_is_never_judged_as_missing() {
    let f = Fixture::new();
    f.session_without_cwd("-Users-someone-gone", &uuid(10))
        .user("cwd 없음")
        .build();

    let result = scan(&f.paths());
    let s = result.sessions().next().unwrap();
    assert_eq!(
        s.project_exists, None,
        "확인 불가는 '없음'이 아니다 (PRD §11.1)"
    );
    assert_eq!(result.projects[0].label, "/Users/someone/gone");
}

#[test]
fn subagent_sessions_are_marked() {
    let f = Fixture::new();
    let p = f.source_tree("agents");
    let parent = f
        .session(p.to_str().unwrap(), &uuid(11))
        .user("부모")
        .build();
    f.session(p.to_str().unwrap(), &uuid(12))
        .subagent_of(&parent)
        .user("하위 작업")
        .build();

    let result = scan(&f.paths());
    let kinds: Vec<_> = result.sessions().map(|s| s.kind).collect();
    assert!(kinds.contains(&SessionKind::Subagent));
    assert!(kinds.contains(&SessionKind::Normal));
}

#[test]
fn orphan_artifacts_appear_as_orphan_sessions() {
    let f = Fixture::new();
    let p = f.source_tree("live");
    f.session(p.to_str().unwrap(), &uuid(20))
        .user("살아있음")
        .build();
    f.orphan_env(&uuid(99));
    f.orphan_task("deadbeef");

    let result = scan(&f.paths());
    let orphans = result
        .projects
        .iter()
        .find(|p| p.key == ORPHAN_KEY)
        .expect("고아 데이터 그룹");
    let ids: Vec<&str> = orphans.sessions.iter().map(|s| s.id.as_str()).collect();
    assert!(ids.contains(&uuid(99).as_str()));
    assert!(ids.contains(&"session-deadbeef"));
    assert!(
        orphans
            .sessions
            .iter()
            .all(|s| s.kind == SessionKind::Orphan)
    );
    assert_eq!(
        orphans.short_label(),
        "고아 데이터",
        "고아 그룹은 이름으로 구분된다"
    );
}

#[test]
fn artifacts_are_attached_and_sizes_summed() {
    let f = Fixture::new();
    let p = f.source_tree("with-extras");
    let id = f
        .session(p.to_str().unwrap(), &uuid(30))
        .user("질문")
        .with_task()
        .with_env()
        .with_file_history()
        .build();

    let result = scan(&f.paths());
    let s = result
        .find(&support::encode_key(p.to_str().unwrap()), &id)
        .unwrap();
    assert!(
        s.file_count() >= 4,
        "기록 + 연결 데이터: {}",
        s.file_count()
    );
    assert!(s.size_bytes > 0);
    assert!(!s.ambiguous_ownership);
}

#[test]
fn sessions_sorted_by_recency_within_project() {
    let f = Fixture::new();
    let p = f.source_tree("sorted");
    f.session(p.to_str().unwrap(), &uuid(41))
        .user("오래된")
        .age_days(90)
        .build();
    f.session(p.to_str().unwrap(), &uuid(42))
        .user("최근")
        .build();

    let result = scan(&f.paths());
    let names: Vec<&str> = result.projects[0]
        .sessions
        .iter()
        .map(|s| s.display_name.as_str())
        .collect();
    assert_eq!(names, vec!["최근", "오래된"]);
}

#[test]
fn orphan_group_is_listed_last() {
    let f = Fixture::new();
    f.orphan_env(&uuid(50));
    let p = f.source_tree("zzz-last-alphabetically");
    f.session(p.to_str().unwrap(), &uuid(51)).user("x").build();

    let result = scan(&f.paths());
    assert_eq!(result.projects.last().unwrap().key, ORPHAN_KEY);
}
