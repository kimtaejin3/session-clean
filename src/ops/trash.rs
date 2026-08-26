//! 휴지통 목록·복원·영구 삭제와 중단된 작업 복구 (PRD §13, FR-11/12/19).
//!
//! 복원 정책의 핵심은 하나다: **덮어쓰지 않는다.** 원래 경로가 비어 있을 때만
//! 되돌리고, 뭔가 있으면 충돌로 표시하고 그 항목만 건너뛴다. 사용자는 충돌 없는
//! 항목만 골라 다시 복원할 수 있다.
//!
//! 모든 연산은 멱등하다. 같은 작업을 두 번 복원해도 두 번째는 아무 일도 하지 않는다.

use crate::logging;
use crate::ops::manifest::{Manifest, ManifestSession, OpStatus};
use crate::ops::{fsutil, history};
use crate::paths::Paths;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct TrashOp {
    pub manifest: Manifest,
    pub dir: PathBuf,
}

impl TrashOp {
    pub fn bytes(&self) -> u64 {
        self.manifest.total_bytes()
    }

    pub fn session_count(&self) -> usize {
        self.manifest.sessions.len()
    }
}

#[derive(Clone, Debug, Default)]
pub struct RestoreOutcome {
    pub restored: Vec<String>,
    /// (세션 이름, 이미 뭔가 있는 경로)
    pub conflicts: Vec<(String, PathBuf)>,
    pub failed: Vec<(String, String)>,
    pub merged_shared: usize,
}

impl RestoreOutcome {
    pub fn is_clean(&self) -> bool {
        self.conflicts.is_empty() && self.failed.is_empty()
    }
}

fn read_ops(paths: &Paths) -> Vec<TrashOp> {
    let mut ops: Vec<TrashOp> = fsutil::list_dir(&paths.trash_dir())
        .into_iter()
        .filter(|p| p.is_dir())
        .filter_map(|dir| {
            Manifest::load(&dir)
                .ok()
                .map(|manifest| TrashOp { manifest, dir })
        })
        .collect();
    // 최신 작업이 위로.
    ops.sort_by(|a, b| b.manifest.op_id.cmp(&a.manifest.op_id));
    ops
}

/// 휴지통 화면에 보여줄, 정상적으로 완료된 작업들.
pub fn list(paths: &Paths) -> Vec<TrashOp> {
    read_ops(paths)
        .into_iter()
        .filter(|o| o.manifest.status == OpStatus::Complete)
        .filter(|o| !o.manifest.sessions.is_empty())
        .collect()
}

/// FR-19: 이전 실행에서 중단된 작업.
pub fn incomplete(paths: &Paths) -> Vec<TrashOp> {
    read_ops(paths)
        .into_iter()
        .filter(|o| matches!(o.manifest.status, OpStatus::Pending | OpStatus::Failed))
        .collect()
}

pub fn total_bytes(ops: &[TrashOp]) -> u64 {
    ops.iter().map(|o| o.bytes()).sum()
}

pub fn find(paths: &Paths, op_id: &str) -> Option<TrashOp> {
    let dir = Manifest::op_dir(paths, op_id);
    Manifest::load(&dir)
        .ok()
        .map(|manifest| TrashOp { manifest, dir })
}

/// 선택한 작업(또는 그 안의 일부 세션)을 원래 위치로 되돌린다.
pub fn restore(
    paths: &Paths,
    op_id: &str,
    session_ids: Option<&[String]>,
) -> Result<RestoreOutcome> {
    let dir = Manifest::op_dir(paths, op_id);
    let mut manifest = Manifest::load(&dir)
        .with_context(|| format!("작업 기록을 읽지 못했습니다: {}", dir.display()))?;
    let mut outcome = RestoreOutcome::default();
    let mut remaining: Vec<ManifestSession> = Vec::new();

    for session in std::mem::take(&mut manifest.sessions) {
        let wanted = session_ids.is_none_or(|ids| ids.contains(&session.session_id));
        if !wanted {
            remaining.push(session);
            continue;
        }
        match restore_session(paths, &dir, &session, &mut outcome) {
            RestoreResult::Done => {}
            RestoreResult::KeepInTrash => remaining.push(session),
        }
    }

    manifest.sessions = remaining;
    if manifest.sessions.is_empty() {
        // PRD §13: 복원 완료 후 빈 휴지통 작업 폴더를 제거한다.
        fsutil::remove_path(&dir)?;
    } else {
        manifest.save(&dir)?;
    }

    logging::info(&format!(
        "restore op={op_id} restored={} conflicts={} failed={}",
        outcome.restored.len(),
        outcome.conflicts.len(),
        outcome.failed.len()
    ));
    Ok(outcome)
}

enum RestoreResult {
    /// 세션을 전부 되돌렸다 — manifest에서 빼도 된다.
    Done,
    /// 충돌이나 실패로 일부가 남았다 — 휴지통에 유지한다.
    KeepInTrash,
}

fn restore_session(
    paths: &Paths,
    dir: &Path,
    session: &ManifestSession,
    outcome: &mut RestoreOutcome,
) -> RestoreResult {
    let mut all_ok = true;

    for file in &session.files {
        let original = PathBuf::from(&file.original);
        let stored = dir.join(&file.stored);

        if !stored.exists() {
            // 이미 되돌렸거나 애초에 옮기지 못한 파일 — 멱등하게 넘어간다.
            if !original.exists() {
                all_ok = false;
                outcome.failed.push((
                    session.display_name.clone(),
                    format!("{} 없음", file.stored),
                ));
            }
            continue;
        }
        if original.exists() {
            // PRD §13: 덮어쓰지 않는다.
            all_ok = false;
            outcome
                .conflicts
                .push((session.display_name.clone(), original));
            continue;
        }
        if fsutil::ensure_within(&paths.claude_dir, &original).is_err() {
            all_ok = false;
            outcome.failed.push((
                session.display_name.clone(),
                format!("안전 검증 실패: {}", original.display()),
            ));
            continue;
        }
        if let Err(e) = fsutil::move_path(&stored, &original) {
            all_ok = false;
            outcome
                .failed
                .push((session.display_name.clone(), format!("{e}")));
        }
    }

    // 공유 기록은 같은 세션 ID가 지금 없을 때만 병합한다.
    match history::merge_back(&paths.history_file(), &session.shared) {
        Ok(n) => outcome.merged_shared += n,
        Err(e) => {
            all_ok = false;
            outcome.failed.push((
                session.display_name.clone(),
                format!("공유 기록 병합 실패: {e}"),
            ));
        }
    }

    if all_ok {
        outcome.restored.push(session.display_name.clone());
        RestoreResult::Done
    } else {
        RestoreResult::KeepInTrash
    }
}

/// FR-12: 휴지통 항목을 영구 삭제한다. 삭제한 바이트를 돌려준다.
pub fn purge(paths: &Paths, op_id: &str, session_ids: Option<&[String]>) -> Result<u64> {
    let dir = Manifest::op_dir(paths, op_id);
    let mut manifest = Manifest::load(&dir)?;
    let mut freed = 0u64;

    if session_ids.is_none() {
        freed = manifest.total_bytes();
        fsutil::remove_path(&dir)?;
        logging::info(&format!("purge op={op_id} whole bytes={freed}"));
        return Ok(freed);
    }
    let ids = session_ids.unwrap();

    let mut remaining = Vec::new();
    for session in std::mem::take(&mut manifest.sessions) {
        if !ids.contains(&session.session_id) {
            remaining.push(session);
            continue;
        }
        for file in &session.files {
            let stored = dir.join(&file.stored);
            // 휴지통 안의 파일만 지운다 — 작업 폴더를 벗어나면 건드리지 않는다.
            if fsutil::ensure_within(&paths.trash_dir(), &stored).is_ok() {
                freed += file.size;
                fsutil::remove_path(&stored)?;
            }
        }
    }
    manifest.sessions = remaining;
    if manifest.sessions.is_empty() {
        fsutil::remove_path(&dir)?;
    } else {
        manifest.save(&dir)?;
    }
    logging::info(&format!("purge op={op_id} partial bytes={freed}"));
    Ok(freed)
}

/// FR-19: 중단된 작업을 되돌린다. 복원과 같은 정책(덮어쓰지 않음)을 쓴다.
pub fn recover(paths: &Paths, op_id: &str) -> Result<RestoreOutcome> {
    let dir = Manifest::op_dir(paths, op_id);
    let mut manifest = Manifest::load(&dir)?;

    // 공유 기록 백업이 남아 있으면 먼저 되돌린다.
    if let Some(backup) = manifest.shared_backup.clone() {
        let backup = PathBuf::from(backup);
        if backup.exists() {
            history::restore_backup(&paths.history_file(), &backup)?;
            history::discard_backup(&backup);
            manifest.shared_backup = None;
        }
    }

    let mut outcome = RestoreOutcome::default();
    let mut remaining = Vec::new();
    for session in std::mem::take(&mut manifest.sessions) {
        match restore_session(paths, &dir, &session, &mut outcome) {
            RestoreResult::Done => {}
            RestoreResult::KeepInTrash => remaining.push(session),
        }
    }
    manifest.sessions = remaining;

    if manifest.sessions.is_empty() {
        fsutil::remove_path(&dir)?;
    } else {
        manifest.status = OpStatus::Failed;
        manifest.save(&dir)?;
    }

    logging::info(&format!(
        "recover op={op_id} restored={} conflicts={}",
        outcome.restored.len(),
        outcome.conflicts.len()
    ));
    Ok(outcome)
}
