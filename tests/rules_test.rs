//! PRD §16 필수 자동 검증 2: 다섯 가지 추천 규칙의 일치·불일치.
//! 실제 fixture 트리를 스캔한 결과에 규칙을 적용해 끝에서 끝까지 확인한다.

mod support;

use sclean::config::Config;
use sclean::live::LiveSessions;
use sclean::rules::{Blocker, Reason, evaluate};
use sclean::scan::{now_secs, scan};
use support::{Fixture, uuid};

fn verdicts(f: &Fixture, cfg: &Config) -> Vec<(String, sclean::rules::Verdict)> {
    let result = scan(&f.paths());
    let now = now_secs();
    let live = LiveSessions::detect(&f.paths());
    result
        .sessions()
        .map(|s| (s.id.clone(), evaluate(s, cfg, now, &live)))
        .collect()
}

fn verdict_for(f: &Fixture, cfg: &Config, id: &str) -> sclean::rules::Verdict {
    verdicts(f, cfg)
        .into_iter()
        .find(|(sid, _)| sid == id)
        .map(|(_, v)| v)
        .expect("세션을 찾지 못했습니다")
}

#[test]
fn r1_old_session_is_recommended_with_an_explanation() {
    let f = Fixture::new();
    let p = f.source_tree("old");
    let id = f
        .session(p.to_str().unwrap(), &uuid(1))
        .user("q1")
        .user("q2")
        .tool_use("Bash")
        .age_days(92)
        .build();

    let v = verdict_for(&f, &Config::default(), &id);
    assert!(v.recommended());
    assert!(matches!(v.reasons[0], Reason::Old { days } if (91..=93).contains(&days)));
    assert!(v.label().contains("마지막 활동 후"));
}

#[test]
fn r1_fresh_session_is_not_recommended() {
    let f = Fixture::new();
    let p = f.source_tree("fresh");
    let id = f
        .session(p.to_str().unwrap(), &uuid(2))
        .user("q1")
        .user("q2")
        .tool_use("Bash")
        .age_days(3)
        .build();

    assert!(!verdict_for(&f, &Config::default(), &id).recommended());
}

#[test]
fn r2_missing_project_is_recommended_and_present_one_is_not() {
    let f = Fixture::new();
    let gone = f.dir.path().join("work/removed");
    let alive = f.source_tree("alive");

    let gone_id = f
        .session(gone.to_str().unwrap(), &uuid(3))
        .user("q1")
        .user("q2")
        .tool_use("Read")
        .age_days(2)
        .build();
    let alive_id = f
        .session(alive.to_str().unwrap(), &uuid(4))
        .user("q1")
        .user("q2")
        .tool_use("Read")
        .age_days(2)
        .build();

    let cfg = Config::default();
    assert!(verdict_for(&f, &cfg, &gone_id).recommended());
    assert!(!verdict_for(&f, &cfg, &alive_id).recommended());
}

#[test]
fn r3_short_session_matches_and_a_worked_session_does_not() {
    let f = Fixture::new();
    let p = f.source_tree("short");
    let short = f
        .session(p.to_str().unwrap(), &uuid(5))
        .user("한 번만 물어봄")
        .assistant("답변")
        .age_days(2)
        .build();
    let worked = f
        .session(p.to_str().unwrap(), &uuid(6))
        .user("작업 요청")
        .tool_use("Edit")
        .age_days(2)
        .build();

    let cfg = Config::default();
    let v = verdict_for(&f, &cfg, &short);
    assert!(v.recommended());
    assert!(matches!(
        v.reasons[0],
        Reason::ShortSession { user_messages: 1 }
    ));
    assert!(!verdict_for(&f, &cfg, &worked).recommended());
}

#[test]
fn r3_ignores_sessions_with_broken_lines() {
    let f = Fixture::new();
    let p = f.source_tree("brokenshort");
    let id = f
        .session(p.to_str().unwrap(), &uuid(7))
        .user("한 번만")
        .raw_line("{깨짐")
        .age_days(2)
        .build();

    let v = verdict_for(&f, &Config::default(), &id);
    assert!(
        !v.reasons
            .iter()
            .any(|r| matches!(r, Reason::ShortSession { .. })),
        "정상 분석된 경우에만 R3를 적용한다"
    );
}

#[test]
fn r4_finished_subagent_matches_but_a_live_one_does_not() {
    let f = Fixture::new();
    let p = f.source_tree("agents");
    let parent = f
        .session(p.to_str().unwrap(), &uuid(8))
        .user("q1")
        .user("q2")
        .tool_use("Task")
        .age_days(2)
        .build();
    let finished = f
        .session(p.to_str().unwrap(), &uuid(9))
        .subagent_of(&parent)
        .user("q1")
        .user("q2")
        .tool_use("Grep")
        .age_days(2)
        .build();
    let working = f
        .session(p.to_str().unwrap(), &uuid(10))
        .subagent_of(&parent)
        .user("q1")
        .user("q2")
        .tool_use("Grep")
        .age_secs(5)
        .build();

    let cfg = Config::default();
    assert!(
        verdict_for(&f, &cfg, &finished)
            .reasons
            .contains(&Reason::FinishedSubagent)
    );
    let live = verdict_for(&f, &cfg, &working);
    assert!(!live.reasons.contains(&Reason::FinishedSubagent));
    assert!(!live.recommended(), "최근 변경 중인 하위 에이전트는 제외");
}

#[test]
fn r5_orphan_data_matches() {
    let f = Fixture::new();
    f.orphan_env_aged(&uuid(11), 40);
    let v = verdict_for(&f, &Config::default(), &uuid(11));
    assert!(v.recommended(), "{:?}", v);
    assert!(
        v.reasons
            .iter()
            .any(|r| matches!(r, Reason::OrphanData { .. }))
    );
    assert!(v.label().contains("대화 기록 없이 남은 데이터"));
}

#[test]
fn disabling_a_rule_removes_its_recommendations() {
    let f = Fixture::new();
    let p = f.source_tree("toggle");
    let id = f
        .session(p.to_str().unwrap(), &uuid(12))
        .user("q1")
        .user("q2")
        .tool_use("Bash")
        .age_days(92)
        .build();

    let mut cfg = Config::default();
    assert!(verdict_for(&f, &cfg, &id).recommended());
    cfg.rule_old = false;
    assert!(!verdict_for(&f, &cfg, &id).recommended());
}

#[test]
fn threshold_change_moves_the_boundary() {
    let f = Fixture::new();
    let p = f.source_tree("threshold");
    let id = f
        .session(p.to_str().unwrap(), &uuid(13))
        .user("q1")
        .user("q2")
        .tool_use("Bash")
        .age_days(45)
        .build();

    let mut cfg = Config::default();
    assert!(
        verdict_for(&f, &cfg, &id).recommended(),
        "30일 기준에서는 추천"
    );
    cfg.old_days = 60;
    assert!(
        !verdict_for(&f, &cfg, &id).recommended(),
        "60일 기준에서는 제외"
    );
}

#[test]
fn unreadable_session_is_blocked_from_cleanup() {
    let f = Fixture::new();
    let p = f.source_tree("unreadable");
    let id = f
        .session(p.to_str().unwrap(), &uuid(14))
        .raw_line("완전히 다른 형식")
        .age_days(400)
        .build();

    let v = verdict_for(&f, &Config::default(), &id);
    assert!(!v.recommended());
    assert!(!v.cleanable());
    assert!(v.blockers.contains(&Blocker::Unparsable));
}

#[test]
fn running_session_is_blocked() {
    let f = Fixture::new();
    let p = f.source_tree("running");
    let id = f
        .session(p.to_str().unwrap(), &uuid(15))
        .user("q1")
        .user("q2")
        .tool_use("Bash")
        .age_days(400)
        .build();
    f.mark_running(&id);

    let v = verdict_for(&f, &Config::default(), &id);
    assert!(v.blockers.contains(&Blocker::Running));
    assert!(!v.cleanable());
}

#[test]
fn ambiguous_prefix_blocks_both_sessions() {
    let f = Fixture::new();
    let p = f.source_tree("ambiguous");
    // 앞 8자가 같은 두 세션이 하나의 tasks/ 폴더를 공유한다.
    let a = "abcd1234-1111-2222-3333-444444444444";
    let b = "abcd1234-9999-8888-7777-666666666666";
    f.session(p.to_str().unwrap(), a)
        .user("q")
        .age_days(90)
        .with_task()
        .build();
    f.session(p.to_str().unwrap(), b)
        .user("q")
        .age_days(90)
        .build();

    let cfg = Config::default();
    let v = verdict_for(&f, &cfg, a);
    assert!(v.blockers.contains(&Blocker::AmbiguousOwnership));
    assert!(!v.cleanable());
}
