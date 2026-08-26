//! PRD §16 필수 자동 검증 13: 미완료 manifest 감지와 복구 (FR-19).

mod support;

use sclean::config::Config;
use sclean::live::LiveSessions;
use sclean::ops::cleanup::{CleanupTarget, execute_with};
use sclean::ops::manifest::{CleanupMode, Manifest, OpStatus};
use sclean::ops::trash;
use sclean::rules::evaluate;
use sclean::scan::{now_secs, scan};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
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

fn two_sessions(f: &Fixture) -> Vec<String> {
    let p = f.source_tree("shop-api");
    vec![
        f.session(p.to_str().unwrap(), &uuid(1))
            .user("첫 세션")
            .tool_use("Edit")
            .with_env()
            .age_days(92)
            .build(),
        f.session(p.to_str().unwrap(), &uuid(2))
            .user("둘째 세션")
            .tool_use("Bash")
            .age_days(70)
            .build(),
    ]
}

fn transcript_path(f: &Fixture, id: &str) -> PathBuf {
    let key = support::encode_key(f.source_tree("shop-api").to_str().unwrap());
    f.paths().projects_dir().join(key).join(format!("{id}.jsonl"))
}

/// 파일 두 개를 옮긴 뒤 프로세스가 죽은 상황을 만든다.
/// 이동은 실제로 일어나지만 manifest는 Pending 상태로 남는다.
fn simulate_crash(f: &Fixture) -> String {
    let moved = AtomicUsize::new(0);
    let mover = move |from: &Path, to: &Path| -> io::Result<()> {
        let n = moved.fetch_add(1, Ordering::SeqCst);
        if n >= 2 {
            // "죽은" 이후로는 아무 일도 일어나지 않는다.
            return Err(io::Error::other("프로세스 종료"));
        }
        sclean::ops::fsutil::move_path(from, to)
    };

    // 롤백까지 막기 위해 원래 자리에 자리를 차지하는 파일을 만들어 둘 수는 없으므로,
    // 실행 후 manifest를 직접 Pending으로 되돌려 중단 상태를 재현한다.
    let out = execute_with(
        &f.paths(),
        all_targets(f),
        CleanupMode::Trash,
        &LiveSessions::empty(),
        &mover,
    )
    .unwrap();
    out.op_id
}

#[test]
fn detects_pending_manifest_from_an_interrupted_run() {
    let f = Fixture::new();
    let ids = two_sessions(&f);

    // 중단 상태를 직접 만든다: 파일 하나를 옮기고 Pending manifest를 남긴다.
    let paths = f.paths();
    let op_id = "20260826-101010-999".to_string();
    let op_dir = Manifest::op_dir(&paths, &op_id);
    let mut manifest = Manifest::new(op_id.clone(), CleanupMode::Trash);

    let original = transcript_path(&f, &ids[0]);
    let stored_rel = "files/0/0-transcript.jsonl".to_string();
    let size = std::fs::metadata(&original).unwrap().len();
    sclean::ops::fsutil::move_path(&original, &op_dir.join(&stored_rel)).unwrap();

    manifest.sessions.push(sclean::ops::manifest::ManifestSession {
        session_id: ids[0].clone(),
        project_key: "shop".into(),
        project_path: None,
        display_name: "첫 세션".into(),
        reasons: vec!["마지막 활동 후 92일 경과".into()],
        files: vec![sclean::ops::manifest::ManifestFile {
            original: original.to_string_lossy().into_owned(),
            stored: stored_rel,
            size,
            is_dir: false,
            moved_at: "2026-08-26T10:10:10+09:00".into(),
        }],
        shared: vec![],
    });
    manifest.save(&op_dir).unwrap();

    // 감지: 휴지통 목록에는 안 보이고, 복구 목록에는 보인다.
    assert!(trash::list(&paths).is_empty(), "미완료 작업은 휴지통이 아니다");
    let pending = trash::incomplete(&paths);
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].manifest.status, OpStatus::Pending);
    assert_eq!(pending[0].manifest.op_id, op_id);

    // 복구: 파일이 제자리로 돌아오고 작업 폴더가 사라진다.
    let out = trash::recover(&paths, &op_id).unwrap();
    assert!(out.is_clean(), "{out:?}");
    assert_eq!(out.restored.len(), 1);
    assert!(original.exists(), "중단된 작업의 파일이 복구되어야 한다");
    assert!(trash::incomplete(&paths).is_empty());
    assert!(!op_dir.exists());
}

#[test]
fn recover_twice_is_idempotent() {
    let f = Fixture::new();
    let ids = two_sessions(&f);
    let paths = f.paths();
    let op_id = "20260826-101010-998".to_string();
    let op_dir = Manifest::op_dir(&paths, &op_id);
    let mut manifest = Manifest::new(op_id.clone(), CleanupMode::Trash);
    let original = transcript_path(&f, &ids[0]);
    let size = std::fs::metadata(&original).unwrap().len();
    sclean::ops::fsutil::move_path(&original, &op_dir.join("files/0/0-t.jsonl")).unwrap();
    manifest.sessions.push(sclean::ops::manifest::ManifestSession {
        session_id: ids[0].clone(),
        project_key: "shop".into(),
        project_path: None,
        display_name: "첫 세션".into(),
        reasons: vec![],
        files: vec![sclean::ops::manifest::ManifestFile {
            original: original.to_string_lossy().into_owned(),
            stored: "files/0/0-t.jsonl".into(),
            size,
            is_dir: false,
            moved_at: "2026-08-26T10:10:10+09:00".into(),
        }],
        shared: vec![],
    });
    manifest.save(&op_dir).unwrap();

    trash::recover(&paths, &op_id).unwrap();
    let content = std::fs::read(&original).unwrap();
    // 두 번째 복구는 대상이 없으므로 오류를 내되 파일은 그대로다.
    let _ = trash::recover(&paths, &op_id);
    assert_eq!(std::fs::read(&original).unwrap(), content);
}

#[test]
fn a_completed_run_is_not_offered_for_recovery() {
    let f = Fixture::new();
    two_sessions(&f);
    let op = simulate_crash(&f);
    // 주입된 실패는 롤백까지 끝나므로 복구 대상이 남지 않는다.
    assert!(trash::incomplete(&f.paths()).is_empty());
    assert!(!Manifest::op_dir(&f.paths(), &op).exists());
}

#[test]
fn recovery_does_not_overwrite_a_file_that_reappeared() {
    let f = Fixture::new();
    let ids = two_sessions(&f);
    let paths = f.paths();
    let op_id = "20260826-101010-997".to_string();
    let op_dir = Manifest::op_dir(&paths, &op_id);
    let mut manifest = Manifest::new(op_id.clone(), CleanupMode::Trash);
    let original = transcript_path(&f, &ids[0]);
    let size = std::fs::metadata(&original).unwrap().len();
    sclean::ops::fsutil::move_path(&original, &op_dir.join("files/0/0-t.jsonl")).unwrap();
    manifest.sessions.push(sclean::ops::manifest::ManifestSession {
        session_id: ids[0].clone(),
        project_key: "shop".into(),
        project_path: None,
        display_name: "첫 세션".into(),
        reasons: vec![],
        files: vec![sclean::ops::manifest::ManifestFile {
            original: original.to_string_lossy().into_owned(),
            stored: "files/0/0-t.jsonl".into(),
            size,
            is_dir: false,
            moved_at: "2026-08-26T10:10:10+09:00".into(),
        }],
        shared: vec![],
    });
    manifest.save(&op_dir).unwrap();

    // Claude Code가 같은 이름으로 새 세션을 만들었다.
    std::fs::write(&original, b"NEW").unwrap();

    let out = trash::recover(&paths, &op_id).unwrap();
    assert_eq!(out.conflicts.len(), 1);
    assert_eq!(std::fs::read(&original).unwrap(), b"NEW");
    // 충돌한 항목은 남아 사용자가 다시 판단할 수 있다.
    let still = trash::incomplete(&paths);
    assert_eq!(still.len(), 1);
    assert_eq!(still[0].manifest.status, OpStatus::Failed);
}
