//! PRD §16 필수 자동 검증 9, 10: 휴지통 세션 복원, 복원 경로 충돌 시 덮어쓰기 방지.

mod support;

use sclean::config::Config;
use sclean::live::LiveSessions;
use sclean::ops::cleanup::{CleanupTarget, execute};
use sclean::ops::manifest::CleanupMode;
use sclean::ops::trash;
use sclean::rules::evaluate;
use sclean::scan::{now_secs, scan};
use support::{Fixture, uuid};

fn all_targets(f: &Fixture) -> Vec<CleanupTarget> {
    let result = scan(&f.paths());
    let now = now_secs();
    let cfg = Config::default();
    let live = LiveSessions::detect(&f.paths());
    result
        .sessions()
        .map(|s| CleanupTarget {
            reasons: evaluate(s, &cfg, now, &live)
                .reasons
                .iter()
                .map(|r| r.label())
                .collect(),
            session: s.clone(),
        })
        .collect()
}

/// 두 세션을 휴지통으로 보내고 (작업 ID, 세션 ID들)을 돌려준다.
fn trashed(f: &Fixture) -> (String, Vec<String>) {
    let p = f.source_tree("shop-api");
    let a = f
        .session(p.to_str().unwrap(), &uuid(1))
        .user("로그인 수정")
        .tool_use("Edit")
        .with_env()
        .age_days(92)
        .build();
    let b = f
        .session(p.to_str().unwrap(), &uuid(2))
        .user("리팩터링")
        .tool_use("Bash")
        .age_days(70)
        .build();
    let out = execute(
        &f.paths(),
        all_targets(f),
        CleanupMode::Trash,
        &LiveSessions::empty(),
    )
    .unwrap();
    assert_eq!(out.succeeded.len(), 2);
    (out.op_id, vec![a, b])
}

fn transcript_path(f: &Fixture, id: &str) -> std::path::PathBuf {
    let key = support::encode_key(f.source_tree("shop-api").to_str().unwrap());
    f.paths().projects_dir().join(key).join(format!("{id}.jsonl"))
}

#[test]
fn restores_session_files_to_original_paths() {
    let f = Fixture::new();
    let (op, ids) = trashed(&f);
    assert!(!transcript_path(&f, &ids[0]).exists());

    let out = trash::restore(&f.paths(), &op, None).unwrap();
    assert!(out.is_clean(), "{out:?}");
    assert_eq!(out.restored.len(), 2);

    for id in &ids {
        assert!(transcript_path(&f, id).exists(), "복원되지 않음: {id}");
    }
    assert!(f.paths().sidecar_dir("session-env").join(&ids[0]).exists());
    // 다시 스캔하면 세션이 그대로 보인다.
    assert_eq!(scan(&f.paths()).session_count(), 2);
    assert!(trash::list(&f.paths()).is_empty(), "빈 작업 폴더는 제거된다");
}

#[test]
fn conflicting_path_is_reported_and_never_overwritten() {
    let f = Fixture::new();
    let (op, ids) = trashed(&f);

    // 같은 자리에 다른 내용의 파일이 생겼다.
    let path = transcript_path(&f, &ids[0]);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"NEW CONTENT").unwrap();

    let out = trash::restore(&f.paths(), &op, None).unwrap();
    assert_eq!(out.conflicts.len(), 1, "{out:?}");
    assert_eq!(out.conflicts[0].1, path);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        b"NEW CONTENT",
        "덮어쓰지 않아야 한다"
    );

    // 충돌 없는 세션은 복원됐고, 충돌한 세션은 휴지통에 남는다.
    assert_eq!(out.restored, vec!["리팩터링".to_string()]);
    let ops = trash::list(&f.paths());
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].session_count(), 1);
    assert_eq!(ops[0].manifest.sessions[0].session_id, ids[0]);
}

#[test]
fn partial_restore_leaves_remaining_sessions_in_trash() {
    let f = Fixture::new();
    let (op, ids) = trashed(&f);

    let out = trash::restore(&f.paths(), &op, Some(&[ids[1].clone()])).unwrap();
    assert_eq!(out.restored.len(), 1);
    assert!(transcript_path(&f, &ids[1]).exists());
    assert!(!transcript_path(&f, &ids[0]).exists());

    let ops = trash::list(&f.paths());
    assert_eq!(ops[0].session_count(), 1);
    assert_eq!(ops[0].manifest.sessions[0].session_id, ids[0]);
}

#[test]
fn restore_is_idempotent_when_run_twice() {
    let f = Fixture::new();
    let (op, ids) = trashed(&f);
    trash::restore(&f.paths(), &op, None).unwrap();
    // 작업 폴더가 사라졌으므로 두 번째는 오류를 돌려주되 데이터는 그대로다.
    assert!(trash::restore(&f.paths(), &op, None).is_err());
    for id in &ids {
        assert!(transcript_path(&f, id).exists());
    }
}

#[test]
fn shared_history_records_are_merged_back_on_restore() {
    let f = Fixture::new();
    let p = f.source_tree("shop-api");
    let a = f
        .session(p.to_str().unwrap(), &uuid(5))
        .user("첫 명령")
        .tool_use("Bash")
        .age_days(92)
        .build();
    f.write_history(&[
        &format!(r#"{{"sessionId":"{a}","display":"첫 명령"}}"#),
        r#"{"sessionId":"other","display":"남을 것"}"#,
    ]);

    let out = execute(
        &f.paths(),
        all_targets(&f),
        CleanupMode::Trash,
        &LiveSessions::empty(),
    )
    .unwrap();
    assert_eq!(f.read_history().len(), 1);

    let r = trash::restore(&f.paths(), &out.op_id, None).unwrap();
    assert_eq!(r.merged_shared, 1);
    let after = f.read_history();
    assert_eq!(after.len(), 2);
    assert!(after.iter().any(|l| l.contains(&a)));
}

#[test]
fn shared_records_are_not_duplicated_if_already_present() {
    let f = Fixture::new();
    let p = f.source_tree("shop-api");
    let a = f
        .session(p.to_str().unwrap(), &uuid(6))
        .user("명령")
        .tool_use("Bash")
        .age_days(92)
        .build();
    f.write_history(&[&format!(r#"{{"sessionId":"{a}","display":"명령"}}"#)]);

    let out = execute(
        &f.paths(),
        all_targets(&f),
        CleanupMode::Trash,
        &LiveSessions::empty(),
    )
    .unwrap();
    // Claude Code가 같은 세션 기록을 다시 써넣은 상황.
    f.write_history(&[&format!(r#"{{"sessionId":"{a}","display":"명령"}}"#)]);

    let r = trash::restore(&f.paths(), &out.op_id, None).unwrap();
    assert_eq!(r.merged_shared, 0, "이미 있으면 병합하지 않는다");
    assert_eq!(f.read_history().len(), 1);
}

#[test]
fn purge_removes_only_the_selected_sessions() {
    let f = Fixture::new();
    let (op, ids) = trashed(&f);

    let freed = trash::purge(&f.paths(), &op, Some(&[ids[0].clone()])).unwrap();
    assert!(freed > 0);
    let ops = trash::list(&f.paths());
    assert_eq!(ops[0].session_count(), 1);
    assert_eq!(ops[0].manifest.sessions[0].session_id, ids[1]);

    // 남은 하나를 복원하면 여전히 정상 동작한다.
    let r = trash::restore(&f.paths(), &op, None).unwrap();
    assert!(r.is_clean());
    assert!(transcript_path(&f, &ids[1]).exists());
    assert!(!transcript_path(&f, &ids[0]).exists(), "영구 삭제된 것은 돌아오지 않는다");
}

#[test]
fn purging_a_whole_operation_empties_the_trash() {
    let f = Fixture::new();
    let (op, _) = trashed(&f);
    let freed = trash::purge(&f.paths(), &op, None).unwrap();
    assert!(freed > 0);
    assert!(trash::list(&f.paths()).is_empty());
    assert!(!sclean::ops::manifest::Manifest::op_dir(&f.paths(), &op).exists());
}

#[test]
fn trash_is_not_expired_automatically() {
    let f = Fixture::new();
    let (op, _) = trashed(&f);
    // 여러 번 목록을 읽어도 그대로 남아있다 (PRD §13).
    for _ in 0..3 {
        assert_eq!(trash::list(&f.paths()).len(), 1);
    }
    assert!(trash::find(&f.paths(), &op).is_some());
}
