//! PRD §16 필수 자동 검증 5,6,7,8,11,12:
//! 일괄 휴지통 이동, 완전 삭제, 중간 실패 롤백, 공유 기록 실패 롤백,
//! 스캔 이후 변경 차단, 프로젝트 소스 경로 제외.

mod support;

use sclean::config::Config;
use sclean::live::LiveSessions;
use sclean::ops::cleanup::{CleanupTarget, SkipReason, execute, execute_with, preview};
use sclean::ops::manifest::{CleanupMode, Manifest, OpStatus};
use sclean::ops::trash;
use sclean::rules::{Blocker, evaluate};
use sclean::scan::{now_secs, scan};
use std::io;
use std::path::Path;
use support::{Fixture, uuid};

fn targets(f: &Fixture) -> Vec<CleanupTarget> {
    let result = scan(&f.paths());
    let now = now_secs();
    let live = LiveSessions::detect(&f.paths());
    let cfg = Config::default();
    result
        .sessions()
        .map(|s| {
            let v = evaluate(s, &cfg, now, &live);
            CleanupTarget {
                session: s.clone(),
                reasons: v.reasons.iter().map(|r| r.label()).collect(),
            }
        })
        .collect()
}

fn target_for(f: &Fixture, id: &str) -> CleanupTarget {
    targets(f)
        .into_iter()
        .find(|t| t.session.id == id)
        .expect("세션 없음")
}

/// 실제 사용자 흐름 그대로: 오래된 세션 두 개 + 연결 데이터.
fn two_old_sessions(f: &Fixture) -> (String, String) {
    let p = f.source_tree("shop-api");
    let a = f
        .session(p.to_str().unwrap(), &uuid(1))
        .user("로그인 수정")
        .tool_use("Edit")
        .with_task()
        .with_env()
        .age_days(92)
        .build();
    let b = f
        .session(p.to_str().unwrap(), &uuid(2))
        .user("리팩터링")
        .tool_use("Bash")
        .with_file_history()
        .age_days(70)
        .build();
    (a, b)
}

#[test]
fn moves_multiple_sessions_to_trash_with_manifest() {
    let f = Fixture::new();
    let (a, b) = two_old_sessions(&f);
    let all = targets(&f);
    let files_before: usize = all.iter().map(|t| t.session.artifacts.len()).sum();

    let out = execute(&f.paths(), all, CleanupMode::Trash, &LiveSessions::empty()).unwrap();
    assert_eq!(out.succeeded.len(), 2, "{out:?}");
    assert!(out.skipped.is_empty());
    assert!(out.is_clean());
    assert_eq!(out.files, files_before);
    assert!(out.bytes > 0);

    // 원본이 사라졌다.
    assert!(
        !f.paths()
            .projects_dir()
            .join(support::encode_key(
                f.source_tree("shop-api").to_str().unwrap()
            ))
            .join(format!("{a}.jsonl"))
            .exists()
    );
    assert!(!f.paths().sidecar_dir("session-env").join(&a).exists());
    assert!(!f.paths().sidecar_dir("file-history").join(&b).exists());

    // 휴지통 작업이 남아있고 완료 상태다.
    let ops = trash::list(&f.paths());
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].manifest.status, OpStatus::Complete);
    assert_eq!(ops[0].session_count(), 2);
    assert!(
        ops[0].manifest.sessions[0]
            .reasons
            .iter()
            .any(|r| r.contains("마지막 활동 후")),
        "추천 이유가 기록되어야 한다"
    );
}

#[test]
fn permanent_delete_removes_the_trash_op_dir() {
    let f = Fixture::new();
    two_old_sessions(&f);
    let out = execute(
        &f.paths(),
        targets(&f),
        CleanupMode::Permanent,
        &LiveSessions::empty(),
    )
    .unwrap();
    assert_eq!(out.succeeded.len(), 2);
    assert!(
        trash::list(&f.paths()).is_empty(),
        "완전 삭제는 흔적을 남기지 않는다"
    );
    assert!(!Manifest::op_dir(&f.paths(), &out.op_id).exists());
}

#[test]
fn failure_midway_rolls_back_every_moved_file() {
    let f = Fixture::new();
    let (a, _b) = two_old_sessions(&f);
    let all = targets(&f);
    let before: Vec<String> = all
        .iter()
        .flat_map(|t| t.session.artifacts.iter())
        .map(|x| x.path.to_string_lossy().into_owned())
        .collect();
    assert!(before.len() >= 4);

    // 세 번째 이동에서 실패시킨다.
    let counter = std::sync::atomic::AtomicUsize::new(0);
    let mover = move |from: &Path, to: &Path| -> io::Result<()> {
        let n = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if n == 2 {
            return Err(io::Error::other("주입된 실패"));
        }
        sclean::ops::fsutil::move_path(from, to)
    };

    let out = execute_with(
        &f.paths(),
        all,
        CleanupMode::Trash,
        &LiveSessions::empty(),
        &mover,
    )
    .unwrap();

    assert!(out.rolled_back, "롤백해야 한다: {out:?}");
    assert!(!out.needs_attention);
    assert!(out.succeeded.is_empty());
    assert_eq!(out.failed.len(), 1);

    // 모든 원본이 제자리에 있다 — 부분 정리 상태가 남으면 안 된다.
    for path in &before {
        assert!(Path::new(path).exists(), "롤백되지 않은 파일: {path}");
    }
    assert!(trash::list(&f.paths()).is_empty());
    assert!(!a.is_empty());
}

#[test]
fn history_replace_failure_rolls_back_whole_operation() {
    let f = Fixture::new();
    let (a, _b) = two_old_sessions(&f);
    f.write_history(&[
        &format!(r#"{{"sessionId":"{a}","display":"로그인"}}"#),
        r#"{"sessionId":"other","display":"남을 것"}"#,
    ]);
    let before = f.read_history();
    let all = targets(&f);
    let originals: Vec<String> = all
        .iter()
        .flat_map(|t| t.session.artifacts.iter())
        .map(|x| x.path.to_string_lossy().into_owned())
        .collect();

    // Claude 루트를 쓰기 불가로 만들면 세션 파일 이동(하위 디렉터리 안에서 일어남)은
    // 성공하지만 history.jsonl 백업 생성은 실패한다 — 6단계만 골라서 실패시키는 방법.
    let root = f.claude();
    set_mode(&root, 0o555);
    let out = execute(&f.paths(), all, CleanupMode::Trash, &LiveSessions::empty());
    set_mode(&root, 0o755);
    let out = out.unwrap();

    assert!(
        out.rolled_back,
        "공유 기록 교체 실패는 전체 롤백이다: {out:?}"
    );
    assert!(out.succeeded.is_empty());
    for path in &originals {
        assert!(Path::new(path).exists(), "롤백되지 않음: {path}");
    }
    assert!(trash::list(&f.paths()).is_empty());
    assert_eq!(f.read_history(), before, "공유 기록도 원상 복구되어야 한다");
}

fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

#[test]
fn shared_history_lines_are_removed_and_recorded() {
    let f = Fixture::new();
    let (a, _b) = two_old_sessions(&f);
    f.write_history(&[
        &format!(r#"{{"sessionId":"{a}","display":"지울 것"}}"#),
        r#"{"sessionId":"keep-me","display":"남을 것"}"#,
    ]);

    execute(
        &f.paths(),
        vec![target_for(&f, &a)],
        CleanupMode::Trash,
        &LiveSessions::empty(),
    )
    .unwrap();

    let after = f.read_history();
    assert_eq!(after.len(), 1);
    assert!(after[0].contains("keep-me"));

    let ops = trash::list(&f.paths());
    assert_eq!(ops[0].manifest.sessions[0].shared.len(), 1);
}

#[test]
fn session_changed_after_scan_is_skipped() {
    let f = Fixture::new();
    let (a, b) = two_old_sessions(&f);
    let all = targets(&f);

    // 스캔 이후 파일이 커진다 (사용자가 세션을 다시 열었다).
    let key = support::encode_key(f.source_tree("shop-api").to_str().unwrap());
    let path = f
        .paths()
        .projects_dir()
        .join(&key)
        .join(format!("{a}.jsonl"));
    let mut content = std::fs::read_to_string(&path).unwrap();
    content.push_str("{\"type\":\"user\",\"message\":{\"content\":\"새 질문\"}}\n");
    std::fs::write(&path, content).unwrap();

    let out = execute(&f.paths(), all, CleanupMode::Trash, &LiveSessions::empty()).unwrap();
    assert_eq!(out.skipped.len(), 1, "{out:?}");
    assert_eq!(out.skipped[0].1, SkipReason::Changed);
    assert!(path.exists(), "변경된 세션은 그대로 남는다");
    assert_eq!(out.succeeded.len(), 1);
    assert!(!b.is_empty());
}

#[test]
fn preview_reports_counts_and_exclusions_before_running() {
    let f = Fixture::new();
    let (a, _b) = two_old_sessions(&f);
    let all = targets(&f);
    f.mark_running(&a);
    let live = LiveSessions::detect(&f.paths());

    let p = preview(&all, &live);
    assert_eq!(p.sessions, 1);
    assert_eq!(p.projects, 1);
    assert!(p.files > 0 && p.bytes > 0);
    assert_eq!(p.excluded.len(), 1);
    assert_eq!(p.excluded[0].1, SkipReason::Blocked(Blocker::Running));
}

#[test]
fn running_session_is_skipped_not_deleted() {
    let f = Fixture::new();
    let (a, _b) = two_old_sessions(&f);
    let all = targets(&f);
    f.mark_running(&a);
    let live = LiveSessions::detect(&f.paths());

    let out = execute(&f.paths(), all, CleanupMode::Trash, &live).unwrap();
    assert_eq!(out.skipped.len(), 1);
    let key = support::encode_key(f.source_tree("shop-api").to_str().unwrap());
    assert!(
        f.paths()
            .projects_dir()
            .join(key)
            .join(format!("{a}.jsonl"))
            .exists(),
        "실행 중 세션은 절대 지우지 않는다"
    );
}

#[test]
fn refuses_targets_outside_the_claude_dir() {
    let f = Fixture::new();
    two_old_sessions(&f);
    let mut all = targets(&f);
    // 버그나 오염된 상태를 흉내낸다: 대상 경로가 Claude 데이터 밖의
    // 프로젝트 소스를 가리킨다. 지문은 정상이라 변경 감지로는 걸리지 않고,
    // 오직 경로 가드만이 이것을 막을 수 있다.
    let outside = f.dir.path().join("work/shop-api/src/main.rs");
    assert!(outside.exists());
    all[0].session.artifacts = vec![sclean::scan::artifacts::Artifact {
        path: outside.clone(),
        kind: sclean::scan::artifacts::ArtifactKind::Transcript,
        is_dir: false,
        size: 12,
        fingerprint: sclean::scan::artifacts::Fingerprint::of(&outside).unwrap(),
    }];

    let err = execute(&f.paths(), all, CleanupMode::Trash, &LiveSessions::empty())
        .expect_err("안전 검증에 걸려야 한다");
    assert!(format!("{err:#}").contains("안전 검증 실패"), "{err:#}");
    assert!(outside.exists(), "프로젝트 소스는 절대 건드리지 않는다");
    assert!(
        trash::list(&f.paths()).is_empty(),
        "검증에 실패하면 아무것도 실행하지 않는다"
    );
}

#[test]
fn project_source_tree_is_untouched_by_a_normal_cleanup() {
    let f = Fixture::new();
    two_old_sessions(&f);
    let source = f.source_tree("shop-api");
    let before = snapshot(&source);

    execute(
        &f.paths(),
        targets(&f),
        CleanupMode::Trash,
        &LiveSessions::empty(),
    )
    .unwrap();

    assert_eq!(before, snapshot(&source), "프로젝트 소스 트리가 변경되었다");
}

#[test]
fn unparsable_session_is_never_cleaned_even_if_selected() {
    let f = Fixture::new();
    let p = f.source_tree("broken");
    let id = f
        .session(p.to_str().unwrap(), &uuid(50))
        .raw_line("알 수 없는 형식")
        .age_days(400)
        .build();

    let out = execute(
        &f.paths(),
        vec![target_for(&f, &id)],
        CleanupMode::Trash,
        &LiveSessions::empty(),
    )
    .unwrap();
    assert_eq!(out.skipped.len(), 1);
    assert_eq!(out.skipped[0].1, SkipReason::Blocked(Blocker::Unparsable));
    assert!(out.succeeded.is_empty());
}

/// 디렉터리의 (상대경로, 내용) 스냅샷.
fn snapshot(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(root, &p, out);
            } else {
                let rel = p.strip_prefix(root).unwrap().to_string_lossy().into_owned();
                out.push((rel, std::fs::read(&p).unwrap()));
            }
        }
    }
    walk(root, root, &mut out);
    out.sort();
    out
}

#[test]
fn selecting_a_parent_and_its_subagent_together_still_succeeds() {
    // 하위 에이전트 기록은 부모 세션 폴더(projects/<p>/<uuid>/subagents/) 안에 있다.
    // 둘 다 고르면 부모 폴더가 먼저 옮겨지면서 하위 에이전트 파일이 사라지고,
    // 순진하게 구현하면 작업 전체가 롤백된다.
    let f = Fixture::new();
    let p = f.source_tree("agents");
    let parent = f
        .session(p.to_str().unwrap(), &uuid(60))
        .user("부모 작업")
        .tool_use("Task")
        .age_days(120)
        .build();
    f.session(p.to_str().unwrap(), &uuid(61))
        .subagent_of(&parent)
        .user("하위 작업")
        .tool_use("Grep")
        .age_days(120)
        .build();

    let all = targets(&f);
    assert_eq!(all.len(), 2);
    let out = execute(&f.paths(), all, CleanupMode::Trash, &LiveSessions::empty()).unwrap();

    assert!(!out.rolled_back, "롤백되면 안 된다: {out:?}");
    assert!(out.failed.is_empty(), "{out:?}");
    assert_eq!(
        out.succeeded.len(),
        1,
        "부모가 하위 에이전트를 포함해 정리한다"
    );
    assert_eq!(out.skipped.len(), 1);
    assert_eq!(out.skipped[0].1, SkipReason::CoveredByAnother);

    // 부모 폴더와 그 안의 하위 에이전트 기록이 모두 사라진다.
    let key = support::encode_key(f.source_tree("agents").to_str().unwrap());
    let proj = f.paths().projects_dir().join(key);
    assert!(!proj.join(format!("{parent}.jsonl")).exists());
    assert!(!proj.join(&parent).exists());

    // 복원하면 둘 다 돌아온다.
    let ops = trash::list(&f.paths());
    let r = trash::restore(&f.paths(), &ops[0].manifest.op_id, None).unwrap();
    assert!(r.is_clean(), "{r:?}");
    assert!(proj.join(format!("{parent}.jsonl")).exists());
    assert!(proj.join(&parent).join("subagents").exists());
    assert_eq!(scan(&f.paths()).session_count(), 2);
}
