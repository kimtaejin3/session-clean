//! 여러 코딩 에이전트 지원.
//!
//! 세션 하나가 파일 하나인 에이전트만 다룬다. 규칙 판정·트랜잭션·휴지통은
//! 에이전트와 무관하게 같은 코드를 쓰므로, 여기서는 **발견·분석·격리**를 본다.

mod support;

use sclean::agents;
use sclean::config::Config;
use sclean::live::LiveSessions;
use sclean::ops::cleanup::{CleanupTarget, execute};
use sclean::ops::manifest::CleanupMode;
use sclean::ops::trash;
use sclean::rules::{Reason, evaluate};
use sclean::scan::{now_secs, scan};
use support::Fixture;

fn uuid(seed: u32) -> String {
    format!("{seed:08x}-1111-2222-3333-444444444444")
}

#[test]
fn registry_lists_the_five_supported_agents() {
    let ids: Vec<&str> = agents::registry().iter().map(|a| a.id()).collect();
    assert_eq!(
        ids,
        vec!["claude", "codex", "gemini", "copilot", "continue"]
    );
}

#[test]
fn every_agent_reports_whether_its_format_was_verified() {
    for a in agents::registry() {
        let verified = a.verified();
        match a.id() {
            "claude" | "codex" | "continue" => {
                assert!(verified, "{} 는 실제 데이터로 확인했다", a.id())
            }
            _ => assert!(!verified, "{} 는 문서 기준이다", a.id()),
        }
    }
}

#[test]
fn absent_agents_are_skipped_without_error() {
    let f = Fixture::new();
    // 아무 에이전트도 설치되지 않은 홈.
    let result = scan(&f.home_paths());
    assert_eq!(result.session_count(), 0);
    assert!(result.errors.is_empty());
}

#[test]
fn discovers_sessions_from_every_installed_agent() {
    let f = Fixture::new();
    let shop = f.source_tree("shop-api");
    f.claude_session_in_home(shop.to_str().unwrap(), &uuid(1), 92);
    f.codex_session(shop.to_str().unwrap(), &uuid(2), 80);
    f.continue_session(shop.to_str().unwrap(), &uuid(3), "컨티뉴 세션", 70);
    f.gemini_session("abc123", "session-1", 60);

    let result = scan(&f.home_paths());
    assert_eq!(result.session_count(), 4);

    let mut found: Vec<&str> = result.sessions().map(|s| s.agent).collect();
    found.sort();
    assert_eq!(found, vec!["claude", "codex", "continue", "gemini"]);
}

#[test]
fn codex_sessions_group_by_the_cwd_in_their_first_line() {
    let f = Fixture::new();
    let shop = f.source_tree("shop-api");
    let blog = f.source_tree("blog");
    f.codex_session(shop.to_str().unwrap(), &uuid(1), 90);
    f.codex_session(blog.to_str().unwrap(), &uuid(2), 90);

    let result = scan(&f.home_paths());
    let labels: Vec<String> = result.projects.iter().map(|p| p.short_label()).collect();
    assert!(labels.contains(&"shop-api".to_string()), "{labels:?}");
    assert!(labels.contains(&"blog".to_string()));
    assert!(result.projects.iter().all(|p| p.exists == Some(true)));
}

#[test]
fn codex_short_session_matches_r3() {
    let f = Fixture::new();
    let shop = f.source_tree("shop-api");
    f.codex_short_session(shop.to_str().unwrap(), &uuid(9), 40);

    let result = scan(&f.home_paths());
    let s = result.sessions().next().unwrap();
    let v = evaluate(s, &Config::default(), now_secs(), &LiveSessions::empty());
    assert!(
        v.reasons
            .iter()
            .any(|r| matches!(r, Reason::ShortSession { user_messages: 1 })),
        "{:?}",
        v.reasons
    );
    assert!(v.recommended());
}

#[test]
fn codex_worked_session_is_not_short() {
    let f = Fixture::new();
    let shop = f.source_tree("shop-api");
    f.codex_session(shop.to_str().unwrap(), &uuid(10), 5);

    let result = scan(&f.home_paths());
    let s = result.sessions().next().unwrap();
    let v = evaluate(s, &Config::default(), now_secs(), &LiveSessions::empty());
    assert!(
        !v.reasons
            .iter()
            .any(|r| matches!(r, Reason::ShortSession { .. }))
    );
}

#[test]
fn continue_uses_its_title_as_the_display_name() {
    let f = Fixture::new();
    let shop = f.source_tree("shop-api");
    f.continue_session(shop.to_str().unwrap(), &uuid(4), "결제 버그 추적", 40);

    let result = scan(&f.home_paths());
    let s = result.sessions().next().unwrap();
    assert_eq!(s.display_name, "결제 버그 추적");
    assert_eq!(s.project_path.as_deref(), Some(shop.as_path()));
}

#[test]
fn gemini_sessions_group_by_hash_and_never_claim_a_missing_project() {
    let f = Fixture::new();
    f.gemini_session("abc123", "session-1", 90);

    let result = scan(&f.home_paths());
    let s = result.sessions().next().unwrap();
    assert_eq!(s.agent, "gemini");
    assert_eq!(
        s.project_exists, None,
        "해시만으로는 프로젝트 존재를 판정할 수 없다"
    );
    let v = evaluate(s, &Config::default(), now_secs(), &LiveSessions::empty());
    assert!(
        !v.reasons
            .iter()
            .any(|r| matches!(r, Reason::MissingProject { .. })),
        "R2 를 적용하면 안 된다"
    );
}

#[test]
fn an_unreadable_session_from_any_agent_is_blocked() {
    let f = Fixture::new();
    let p = f.home_paths();
    let dir = p.agent_root("codex").join("sessions/2026/09/04");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("rollout-2026-09-04T20-00-00-broken.jsonl"),
        "알 수 없는 형식",
    )
    .unwrap();

    let result = scan(&p);
    let s = result.sessions().next().unwrap();
    let v = evaluate(s, &Config::default(), now_secs(), &LiveSessions::empty());
    assert!(!v.cleanable(), "형식을 모르면 지우지 않는다");
    assert!(s.display_name.starts_with("Unparseable"));
}

#[test]
fn cleaning_one_agent_never_touches_another() {
    let f = Fixture::new();
    let shop = f.source_tree("shop-api");
    let claude_file = f.claude_session_in_home(shop.to_str().unwrap(), &uuid(1), 92);
    let codex_file = f.codex_session(shop.to_str().unwrap(), &uuid(2), 92);
    let continue_file = f.continue_session(shop.to_str().unwrap(), &uuid(3), "컨티뉴", 92);

    let paths = f.home_paths();
    let result = scan(&paths);
    let cfg = Config::default();
    let now = now_secs();

    // Codex 세션만 고른다.
    let targets: Vec<CleanupTarget> = result
        .sessions()
        .filter(|s| s.agent == "codex")
        .map(|s| CleanupTarget {
            reasons: evaluate(s, &cfg, now, &LiveSessions::empty())
                .reasons
                .iter()
                .map(|r| r.label())
                .collect(),
            session: s.clone(),
        })
        .collect();
    assert_eq!(targets.len(), 1);

    let out = execute(&paths, targets, CleanupMode::Trash, &LiveSessions::empty()).unwrap();
    assert_eq!(out.succeeded.len(), 1, "{out:?}");

    assert!(!codex_file.exists(), "Codex 세션은 옮겨져야 한다");
    assert!(claude_file.exists(), "Claude 세션은 그대로여야 한다");
    assert!(continue_file.exists(), "Continue 세션은 그대로여야 한다");

    // 작업 기록에 어느 에이전트였는지 남는다.
    let ops = trash::list(&paths);
    assert_eq!(ops[0].manifest.sessions[0].agent, "codex");
}

#[test]
fn a_trashed_session_from_another_agent_restores_correctly() {
    let f = Fixture::new();
    let shop = f.source_tree("shop-api");
    let codex_file = f.codex_session(shop.to_str().unwrap(), &uuid(2), 92);
    let paths = f.home_paths();

    let result = scan(&paths);
    let targets: Vec<CleanupTarget> = result
        .sessions()
        .map(|s| CleanupTarget {
            reasons: vec![],
            session: s.clone(),
        })
        .collect();
    let out = execute(&paths, targets, CleanupMode::Trash, &LiveSessions::empty()).unwrap();
    assert!(!codex_file.exists());

    let r = trash::restore(&paths, &out.op_id, None).unwrap();
    assert!(r.is_clean(), "{r:?}");
    assert!(codex_file.exists(), "원래 자리로 돌아와야 한다");
    assert_eq!(scan(&paths).session_count(), 1);
}

#[test]
fn a_target_outside_its_own_agent_root_is_refused() {
    let f = Fixture::new();
    let shop = f.source_tree("shop-api");
    f.codex_session(shop.to_str().unwrap(), &uuid(2), 92);
    let paths = f.home_paths();

    let result = scan(&paths);
    let mut targets: Vec<CleanupTarget> = result
        .sessions()
        .map(|s| CleanupTarget {
            reasons: vec![],
            session: s.clone(),
        })
        .collect();

    // Codex 세션이 Claude 디렉터리의 파일을 가리키게 만든다.
    let intruder = f.claude_session_in_home(shop.to_str().unwrap(), &uuid(1), 10);
    targets[0].session.artifacts = vec![sclean::scan::artifacts::Artifact {
        path: intruder.clone(),
        kind: sclean::scan::artifacts::ArtifactKind::Transcript,
        is_dir: false,
        size: 10,
        fingerprint: sclean::scan::artifacts::Fingerprint::of(&intruder).unwrap(),
    }];

    let err = execute(&paths, targets, CleanupMode::Trash, &LiveSessions::empty())
        .expect_err("자기 에이전트 루트 밖은 거부해야 한다");
    assert!(
        format!("{err:#}").contains("safety check failed"),
        "{err:#}"
    );
    assert!(intruder.exists());
}

#[test]
fn projects_are_ordered_by_agent_then_name() {
    let f = Fixture::new();
    let shop = f.source_tree("shop-api");
    f.continue_session(shop.to_str().unwrap(), &uuid(3), "컨티뉴", 40);
    f.codex_session(shop.to_str().unwrap(), &uuid(2), 40);
    f.claude_session_in_home(shop.to_str().unwrap(), &uuid(1), 40);

    let result = scan(&f.home_paths());
    let order: Vec<&str> = result.projects.iter().map(|p| p.agent).collect();
    assert_eq!(
        order,
        vec!["claude", "codex", "continue"],
        "레지스트리 순서"
    );
}
