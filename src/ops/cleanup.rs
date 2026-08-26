//! 정리 트랜잭션 (PRD §12).
//!
//! 휴지통 이동과 완전 삭제는 **같은 절차**를 쓴다. 차이는 마지막 단계에서
//! 작업 폴더를 남기느냐 지우느냐 뿐이다. 절차가 하나라 안전 검증도 한 곳에만 있다.
//!
//! ```text
//! 1. 재스캔·지문 대조로 변경된 세션 제외        (FR-13)
//! 2. 실행 중 세션 제외
//! 3. 모든 대상이 claude_dir 안에 있는지 검증    (FR-16) — 하나라도 실패하면 전체 중단
//! 4. Pending manifest 저장                      (강제 종료 대비)
//! 5. 세션 전용 파일 이동
//! 6. 공유 기록 백업 후 원자적 교체
//! 7. manifest를 Complete로 갱신
//! 8. Trash면 보존 / 9. Permanent면 작업 폴더 삭제
//! ```
//!
//! 5~6단계에서 실패하면 이동한 파일과 공유 기록을 역순으로 되돌린다 (FR-14).

use crate::live::LiveSessions;
use crate::logging;
use crate::ops::manifest::{
    CleanupMode, FILES_DIR, Manifest, ManifestFile, ManifestSession, OpStatus, new_op_id,
};
use crate::ops::{fsutil, history};
use crate::paths::Paths;
use crate::rules::Blocker;
use crate::scan::session::Session;
use anyhow::{Context, Result};
use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct CleanupTarget {
    pub session: Session,
    /// 화면에 보여줬던 추천 이유. manifest에 그대로 기록한다 (PRD §11.2).
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SkipReason {
    /// 스캔 이후 크기나 수정 시각이 바뀌었다.
    Changed,
    /// 스캔 이후 파일이 사라졌다.
    Missing,
    /// 이 세션의 파일이 함께 정리되는 다른 세션의 폴더 안에 있다.
    /// 하위 에이전트 기록이 부모 세션 폴더 안에 있는 경우가 그렇다.
    CoveredByAnother,
    Blocked(Blocker),
}

impl SkipReason {
    pub fn label(&self) -> String {
        match self {
            SkipReason::Changed => "스캔 이후 변경됨".into(),
            SkipReason::Missing => "파일이 이미 없음".into(),
            SkipReason::CoveredByAnother => "상위 세션과 함께 정리됨".into(),
            SkipReason::Blocked(b) => b.label().to_string(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct CleanupPreview {
    pub projects: usize,
    pub sessions: usize,
    pub files: usize,
    pub bytes: u64,
    /// 스캔 이후 변경되어 제외된 세션 (PRD §8.2).
    pub excluded: Vec<(String, SkipReason)>,
}

#[derive(Clone, Debug, Default)]
pub struct CleanupOutcome {
    pub op_id: String,
    pub mode: Option<CleanupMode>,
    pub succeeded: Vec<String>,
    pub skipped: Vec<(String, SkipReason)>,
    pub failed: Vec<(String, String)>,
    pub bytes: u64,
    pub files: usize,
    pub rolled_back: bool,
    /// 롤백까지 실패해 사람이 봐야 하는 상태.
    pub needs_attention: bool,
}

impl CleanupOutcome {
    pub fn is_clean(&self) -> bool {
        self.failed.is_empty() && !self.needs_attention
    }
}

/// 파일 이동 방식. 테스트가 실패를 주입할 수 있도록 분리했다.
pub type Mover = dyn Fn(&Path, &Path) -> io::Result<()> + Send + Sync;

fn real_mover(from: &Path, to: &Path) -> io::Result<()> {
    fsutil::move_path(from, to)
}

/// 실행 전에 보여줄 요약 (FR-08). 여기서도 지문을 대조해 제외 목록을 만든다.
pub fn preview(targets: &[CleanupTarget], live: &LiveSessions) -> CleanupPreview {
    let mut p = CleanupPreview::default();
    let mut projects = HashSet::new();
    let (ready, excluded) = partition(targets, live);
    p.excluded = excluded;
    for t in ready {
        projects.insert(t.session.project_key.clone());
        p.sessions += 1;
        p.files += t.session.artifacts.len();
        p.bytes += t.session.size_bytes;
    }
    p.projects = projects.len();
    p
}

/// 실행 가능한 대상과 제외 목록으로 나눈다.
///
/// `preview`와 `execute`가 반드시 같은 판단을 쓰도록 한 곳에 모았다 —
/// 확인 화면에 보인 세션 수와 실제로 실행되는 것이 어긋나면 안 된다.
fn partition<'a>(
    targets: &'a [CleanupTarget],
    live: &LiveSessions,
) -> (Vec<&'a CleanupTarget>, Vec<(String, SkipReason)>) {
    let mut ready: Vec<&CleanupTarget> = Vec::new();
    let mut excluded = Vec::new();
    for t in targets {
        match skip_reason(t, live) {
            Some(r) => excluded.push((t.session.display_name.clone(), r)),
            None => ready.push(t),
        }
    }

    // 하위 에이전트 기록은 부모 세션 폴더(projects/<p>/<uuid>/subagents/) 안에 있다.
    // 부모를 옮기면 함께 옮겨지므로, 따로 옮기려 들면 "파일이 없다"며 작업 전체가
    // 롤백된다. 부모가 같은 작업에 들어 있으면 그 세션은 부모에게 맡기고 제외한다.
    let paths: Vec<Vec<PathBuf>> = ready
        .iter()
        .map(|t| t.session.artifacts.iter().map(|a| a.path.clone()).collect())
        .collect();
    let mut keep = Vec::new();
    for (i, target) in ready.iter().enumerate() {
        let covered = !paths[i].is_empty()
            && paths[i].iter().all(|path| {
                paths.iter().enumerate().any(|(j, other)| {
                    j != i && other.iter().any(|o| path != o && path.starts_with(o))
                })
            });
        if covered {
            excluded.push((
                target.session.display_name.clone(),
                SkipReason::CoveredByAnother,
            ));
        } else {
            keep.push(*target);
        }
    }
    (keep, excluded)
}

/// 이 세션을 정리에서 제외해야 하는가.
fn skip_reason(t: &CleanupTarget, live: &LiveSessions) -> Option<SkipReason> {
    if live.contains(&t.session.id) {
        return Some(SkipReason::Blocked(Blocker::Running));
    }
    if t.session.ambiguous_ownership {
        return Some(SkipReason::Blocked(Blocker::AmbiguousOwnership));
    }
    if t.session.analysis.is_unreadable() {
        return Some(SkipReason::Blocked(Blocker::Unparsable));
    }
    if t.session.artifacts.is_empty() {
        return Some(SkipReason::Blocked(Blocker::NothingToClean));
    }
    // FR-13: 스캔 이후 변경 감지.
    for a in &t.session.artifacts {
        if !a.still_exists() {
            return Some(SkipReason::Missing);
        }
        if !a.unchanged() {
            return Some(SkipReason::Changed);
        }
    }
    None
}

pub fn execute(
    paths: &Paths,
    targets: Vec<CleanupTarget>,
    mode: CleanupMode,
    live: &LiveSessions,
) -> Result<CleanupOutcome> {
    execute_with(paths, targets, mode, live, &real_mover)
}

pub fn execute_with(
    paths: &Paths,
    targets: Vec<CleanupTarget>,
    mode: CleanupMode,
    live: &LiveSessions,
    mover: &Mover,
) -> Result<CleanupOutcome> {
    let mut outcome = CleanupOutcome {
        mode: Some(mode),
        ..Default::default()
    };

    // --- 1~2단계: 재검증 ---
    let (ready, excluded) = partition(&targets, live);
    outcome.skipped = excluded;
    if ready.is_empty() {
        logging::info("cleanup aborted: no eligible sessions");
        return Ok(outcome);
    }

    // --- 3단계: 경로 검증. 하나라도 밖에 있으면 아무것도 하지 않는다 (FR-16) ---
    for t in &ready {
        for a in &t.session.artifacts {
            fsutil::ensure_within(&paths.claude_dir, &a.path).with_context(|| {
                format!(
                    "안전 검증 실패 — 작업을 실행하지 않았습니다 (세션 {})",
                    t.session.id
                )
            })?;
        }
    }

    // --- 4단계: Pending manifest 선기록 ---
    let op_id = new_op_id();
    let op_dir = Manifest::op_dir(paths, &op_id);
    outcome.op_id = op_id.clone();
    let mut manifest = Manifest::new(op_id.clone(), mode);

    let session_ids: HashSet<String> = ready.iter().map(|t| t.session.id.clone()).collect();
    let history_plan = history::plan_removal(&paths.history_file(), &session_ids)?;

    for (i, t) in ready.iter().enumerate() {
        let mut files = Vec::new();
        for (j, a) in t.session.artifacts.iter().enumerate() {
            let name = a
                .path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| format!("entry-{j}"));
            files.push(ManifestFile {
                original: a.path.to_string_lossy().into_owned(),
                // 세션마다 폴더를 나눠 이름 충돌을 원천 차단한다.
                stored: format!("{FILES_DIR}/{i}/{j}-{name}"),
                size: a.size,
                is_dir: a.is_dir,
                moved_at: String::new(),
            });
        }
        manifest.sessions.push(ManifestSession {
            session_id: t.session.id.clone(),
            project_key: t.session.project_key.clone(),
            project_path: t
                .session
                .project_path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            display_name: t.session.display_name.clone(),
            reasons: t.reasons.clone(),
            files,
            shared: history_plan
                .removed
                .iter()
                .filter(|r| owns_record(&t.session.id, r))
                .cloned()
                .collect(),
        });
    }

    manifest
        .save(&op_dir)
        .with_context(|| format!("작업 기록을 저장하지 못했습니다: {}", op_dir.display()))?;
    logging::info(&format!(
        "cleanup start op={op_id} mode={:?} sessions={} files={}",
        mode,
        manifest.sessions.len(),
        manifest.total_files()
    ));

    // --- 5단계: 이동 ---
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    let now = chrono::Local::now().to_rfc3339();
    let mut failure: Option<String> = None;

    'outer: for si in 0..manifest.sessions.len() {
        for fi in 0..manifest.sessions[si].files.len() {
            let original = PathBuf::from(&manifest.sessions[si].files[fi].original);
            let stored = op_dir.join(&manifest.sessions[si].files[fi].stored);
            match mover(&original, &stored) {
                Ok(()) => {
                    manifest.sessions[si].files[fi].moved_at = now.clone();
                    moved.push((original, stored));
                }
                Err(e) => {
                    failure = Some(format!("{} 이동 실패: {e}", original.display()));
                    break 'outer;
                }
            }
        }
    }

    // --- 6단계: 공유 기록 교체 ---
    let mut backup: Option<PathBuf> = None;
    if failure.is_none() {
        match history::apply(&paths.history_file(), &history_plan, &op_id) {
            Ok(b) => {
                backup = b.clone();
                manifest.shared_backup = b.map(|p| p.to_string_lossy().into_owned());
            }
            Err(e) => failure = Some(format!("공유 기록 교체 실패: {e}")),
        }
    }

    // --- 실패 시 롤백 (FR-14) ---
    if let Some(msg) = failure {
        logging::error(&format!("cleanup failed op={op_id}: {msg}"));
        let recovered = rollback(
            &mut manifest,
            &moved,
            backup.as_deref(),
            &paths.history_file(),
        );
        manifest.status = if recovered {
            OpStatus::RolledBack
        } else {
            OpStatus::Failed
        };
        let _ = manifest.save(&op_dir);
        outcome.rolled_back = recovered;
        outcome.needs_attention = !recovered;
        outcome.failed.push(("정리 작업".to_string(), msg));
        for t in &ready {
            outcome
                .skipped
                .push((t.session.display_name.clone(), SkipReason::Changed));
        }
        outcome.skipped.retain(|(n, _)| {
            // 롤백했으므로 성공 목록은 비어 있다. 중복 표기를 정리한다.
            !outcome.succeeded.contains(n)
        });
        if recovered {
            // 되돌렸으면 빈 작업 폴더는 남길 필요가 없다.
            let _ = fsutil::remove_path(&op_dir);
        }
        return Ok(outcome);
    }

    // --- 7단계: 완료 표시 ---
    manifest.status = OpStatus::Complete;
    manifest.save(&op_dir)?;
    if let Some(b) = &backup {
        history::discard_backup(b);
    }

    outcome.bytes = manifest.total_bytes();
    outcome.files = manifest.total_files();
    outcome.succeeded = manifest
        .sessions
        .iter()
        .map(|s| s.display_name.clone())
        .collect();

    // --- 8~9단계 ---
    if mode == CleanupMode::Permanent {
        fsutil::remove_path(&op_dir)
            .with_context(|| format!("작업 폴더를 지우지 못했습니다: {}", op_dir.display()))?;
    }

    logging::info(&format!(
        "cleanup complete op={op_id} mode={:?} sessions={} bytes={}",
        mode,
        outcome.succeeded.len(),
        outcome.bytes
    ));
    Ok(outcome)
}

/// 이미 옮긴 파일과 공유 기록을 원상 복구한다. 전부 성공했을 때만 true.
fn rollback(
    manifest: &mut Manifest,
    moved: &[(PathBuf, PathBuf)],
    backup: Option<&Path>,
    history_file: &Path,
) -> bool {
    let mut ok = true;
    if let Some(b) = backup
        && history::restore_backup(history_file, b).is_err()
    {
        ok = false;
    }
    // 역순으로 되돌린다.
    for (original, stored) in moved.iter().rev() {
        if original.exists() {
            // 원래 자리에 뭔가 생겼다 — 덮어쓰지 않는다 (PRD §13 원칙과 동일).
            ok = false;
            logging::error(&format!(
                "rollback conflict: {} already exists",
                original.display()
            ));
            continue;
        }
        if fsutil::move_path(stored, original).is_err() {
            ok = false;
            logging::error(&format!("rollback failed: {}", original.display()));
        }
    }
    if ok && let Some(b) = backup {
        history::discard_backup(b);
    }
    if ok {
        for s in &mut manifest.sessions {
            for f in &mut s.files {
                f.moved_at.clear();
            }
        }
    }
    ok
}

fn owns_record(session_id: &str, r: &crate::ops::manifest::SharedRecord) -> bool {
    serde_json::from_str::<serde_json::Value>(&r.content)
        .ok()
        .and_then(|v| {
            v.get("sessionId")
                .or_else(|| v.get("session_id"))
                .and_then(|s| s.as_str())
                .map(|s| s.to_string())
        })
        .is_some_and(|id| id == session_id)
}
