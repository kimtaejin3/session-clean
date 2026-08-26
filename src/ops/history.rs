//! 공유 기록(`history.jsonl`) 처리 (PRD §12-6, §13).
//!
//! 이 파일은 여러 세션이 함께 쓰는 유일한 대상이라 파일을 통째로 옮길 수 없다.
//! 그래서 "백업 → 임시 파일에 재작성 → 원자적 교체" 절차를 쓴다.
//!
//! **소유권 원칙:** 줄에 `sessionId`가 없으면 어느 세션의 것인지 확정할 수 없다.
//! PRD §9의 "연결 데이터의 소유 세션을 확정할 수 없음"에 해당하므로, 그런 줄은
//! 절대 지우지 않는다. 파일 전체에 `sessionId`가 하나도 없으면 아무것도 하지 않는다.

use crate::ops::fsutil;
use crate::ops::manifest::SharedRecord;
use anyhow::Result;
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub struct HistoryPlan {
    pub removed: Vec<SharedRecord>,
    pub kept: Vec<String>,
    /// 파일이 존재하고 세션 소유를 판별할 수 있었는가.
    pub applicable: bool,
}

impl HistoryPlan {
    pub fn is_noop(&self) -> bool {
        self.removed.is_empty()
    }
}

pub fn plan_removal(history: &Path, session_ids: &HashSet<String>) -> Result<HistoryPlan> {
    let Ok(text) = std::fs::read_to_string(history) else {
        // 파일이 없으면 할 일이 없다 (PRD §11.1).
        return Ok(HistoryPlan::default());
    };
    let mut plan = HistoryPlan::default();
    let mut saw_session_field = false;

    for (idx, line) in text.lines().enumerate() {
        let owner = serde_json::from_str::<Value>(line)
            .ok()
            .and_then(|v| session_id_of(&v));
        if owner.is_some() {
            saw_session_field = true;
        }
        match owner {
            Some(id) if session_ids.contains(&id) => plan.removed.push(SharedRecord {
                file: history.to_string_lossy().into_owned(),
                line_index: idx,
                content: line.to_string(),
            }),
            // 파싱 실패한 줄과 소유가 불명확한 줄은 항상 보존한다.
            _ => plan.kept.push(line.to_string()),
        }
    }

    plan.applicable = saw_session_field;
    if !plan.applicable {
        // 소유를 확정할 수 없으므로 손대지 않는다.
        plan.removed.clear();
        plan.kept.clear();
    }
    Ok(plan)
}

fn session_id_of(v: &Value) -> Option<String> {
    for key in ["sessionId", "session_id"] {
        if let Some(s) = v.get(key).and_then(Value::as_str)
            && !s.is_empty()
        {
            return Some(s.to_string());
        }
    }
    None
}

/// 백업을 만들고 남길 줄만 원자적으로 교체한다. 백업 경로를 돌려준다.
pub fn apply(history: &Path, plan: &HistoryPlan, op_id: &str) -> Result<Option<PathBuf>> {
    if plan.is_noop() {
        return Ok(None);
    }
    let backup = history.with_file_name(format!(
        "{}.sclean-bak-{op_id}",
        fsutil_file_name(history)
    ));
    std::fs::copy(history, &backup)?;

    let mut body = plan.kept.join("\n");
    if !body.is_empty() {
        body.push('\n');
    }
    fsutil::atomic_write(history, body.as_bytes())?;
    Ok(Some(backup))
}

pub fn restore_backup(history: &Path, backup: &Path) -> Result<()> {
    let bytes = std::fs::read(backup)?;
    fsutil::atomic_write(history, &bytes)?;
    Ok(())
}

pub fn discard_backup(backup: &Path) {
    let _ = std::fs::remove_file(backup);
}

/// 복원 시 병합 (PRD §13): 현재 내용에 같은 세션 ID가 없을 때만 되돌린다.
pub fn merge_back(history: &Path, records: &[SharedRecord]) -> Result<usize> {
    if records.is_empty() {
        return Ok(0);
    }
    let current = std::fs::read_to_string(history).unwrap_or_default();
    let present: HashSet<String> = current
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter_map(|v| session_id_of(&v))
        .collect();

    let mut lines: Vec<String> = current.lines().map(|s| s.to_string()).collect();
    let mut merged = 0;
    for r in records {
        let owner = serde_json::from_str::<Value>(&r.content)
            .ok()
            .and_then(|v| session_id_of(&v));
        match owner {
            Some(id) if present.contains(&id) => continue, // 이미 있다 — 덮어쓰지 않는다
            _ => {
                // 원래 위치를 최대한 존중하되, 파일이 짧아졌으면 끝에 붙인다.
                let at = r.line_index.min(lines.len());
                lines.insert(at, r.content.clone());
                merged += 1;
            }
        }
    }

    let mut body = lines.join("\n");
    if !body.is_empty() {
        body.push('\n');
    }
    fsutil::atomic_write(history, body.as_bytes())?;
    Ok(merged)
}

fn fsutil_file_name(p: &Path) -> String {
    p.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "history.jsonl".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[&str]) -> HashSet<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn write(lines: &[&str]) -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("history.jsonl");
        std::fs::write(&p, format!("{}\n", lines.join("\n"))).unwrap();
        (tmp, p)
    }

    #[test]
    fn removes_only_lines_owned_by_target_sessions() {
        let (_t, p) = write(&[
            r#"{"sessionId":"a","display":"첫 명령"}"#,
            r#"{"sessionId":"b","display":"남을 것"}"#,
            r#"{"sessionId":"a","display":"두번째"}"#,
        ]);
        let plan = plan_removal(&p, &ids(&["a"])).unwrap();
        assert_eq!(plan.removed.len(), 2);
        assert_eq!(plan.kept.len(), 1);
        apply(&p, &plan, "op1").unwrap();
        let after = std::fs::read_to_string(&p).unwrap();
        assert!(after.contains(r#""sessionId":"b""#));
        assert!(!after.contains(r#""display":"첫 명령""#));
    }

    #[test]
    fn keeps_everything_when_no_line_carries_session_id() {
        let (_t, p) = write(&[
            r#"{"display":"명령 1","project":"/w"}"#,
            r#"{"display":"명령 2","project":"/w"}"#,
        ]);
        let plan = plan_removal(&p, &ids(&["a"])).unwrap();
        assert!(!plan.applicable);
        assert!(plan.is_noop(), "소유를 확정할 수 없으면 손대지 않는다");
        let before = std::fs::read_to_string(&p).unwrap();
        assert!(apply(&p, &plan, "op1").unwrap().is_none());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), before);
    }

    #[test]
    fn preserves_unparsable_lines() {
        let (_t, p) = write(&[
            r#"{"sessionId":"a","display":"지울 것"}"#,
            "깨진 줄",
            r#"{"sessionId":"b"}"#,
        ]);
        let plan = plan_removal(&p, &ids(&["a"])).unwrap();
        apply(&p, &plan, "op1").unwrap();
        let after = std::fs::read_to_string(&p).unwrap();
        assert!(after.contains("깨진 줄"), "이해 못 한 줄은 보존한다");
    }

    #[test]
    fn missing_history_file_is_a_noop() {
        let tmp = tempfile::tempdir().unwrap();
        let plan = plan_removal(&tmp.path().join("nope.jsonl"), &ids(&["a"])).unwrap();
        assert!(plan.is_noop());
    }

    #[test]
    fn restore_backup_recovers_original_bytes() {
        let (_t, p) = write(&[r#"{"sessionId":"a"}"#, r#"{"sessionId":"b"}"#]);
        let before = std::fs::read(&p).unwrap();
        let plan = plan_removal(&p, &ids(&["a"])).unwrap();
        let backup = apply(&p, &plan, "op1").unwrap().unwrap();
        assert_ne!(std::fs::read(&p).unwrap(), before);
        restore_backup(&p, &backup).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), before);
    }

    #[test]
    fn merge_back_restores_removed_records() {
        let (_t, p) = write(&[r#"{"sessionId":"a"}"#, r#"{"sessionId":"b"}"#]);
        let plan = plan_removal(&p, &ids(&["a"])).unwrap();
        let removed = plan.removed.clone();
        apply(&p, &plan, "op1").unwrap();
        assert_eq!(merge_back(&p, &removed).unwrap(), 1);
        assert!(std::fs::read_to_string(&p).unwrap().contains(r#""sessionId":"a""#));
    }

    #[test]
    fn merge_back_skips_records_already_present() {
        let (_t, p) = write(&[r#"{"sessionId":"a"}"#]);
        let record = SharedRecord {
            file: p.to_string_lossy().into_owned(),
            line_index: 0,
            content: r#"{"sessionId":"a"}"#.into(),
        };
        assert_eq!(merge_back(&p, &[record]).unwrap(), 0, "덮어쓰지 않는다");
        assert_eq!(std::fs::read_to_string(&p).unwrap().lines().count(), 1);
    }
}
