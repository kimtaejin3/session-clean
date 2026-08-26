//! 정리 작업 기록 (PRD §11.2, §12).
//!
//! manifest는 정리를 **시작하기 전에** `Pending` 상태로 먼저 저장된다.
//! 도중에 강제 종료되어도 다음 실행에서 이 파일을 보고 무엇을 되돌려야 하는지 알 수 있다.
//!
//! 세션 대화 내용 자체는 복제하지 않는다. 공유 기록(`history.jsonl`)에서 지운 줄만
//! 복원을 위해 그대로 보관한다.

use crate::ops::fsutil;
use crate::paths::Paths;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const MANIFEST_FILE: &str = "manifest.json";
pub const FILES_DIR: &str = "files";
pub const MANIFEST_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum OpStatus {
    /// 이동을 시작했지만 아직 끝나지 않았다.
    Pending,
    /// 모든 단계가 성공했다.
    Complete,
    /// 실패해서 원상 복구했다.
    RolledBack,
    /// 실패했고 복구도 실패했다 — 사람이 봐야 한다.
    Failed,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum CleanupMode {
    /// 기본값. 되돌릴 수 있는 쪽이 언제나 기본이어야 한다 (PRD §6).
    #[default]
    Trash,
    Permanent,
}

impl CleanupMode {
    pub fn label(&self) -> &'static str {
        match self {
            CleanupMode::Trash => "휴지통 이동",
            CleanupMode::Permanent => "완전 삭제",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ManifestFile {
    /// 원래 있던 절대 경로.
    pub original: String,
    /// 작업 폴더 기준 상대 경로 (`files/0/xxx.jsonl`).
    pub stored: String,
    pub size: u64,
    pub is_dir: bool,
    pub moved_at: String,
}

/// `history.jsonl`처럼 여러 세션이 공유하는 파일에서 제거한 줄.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SharedRecord {
    pub file: String,
    pub line_index: usize,
    pub content: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ManifestSession {
    pub session_id: String,
    pub project_key: String,
    pub project_path: Option<String>,
    pub display_name: String,
    /// 적용된 추천 이유 (PRD §11.2).
    pub reasons: Vec<String>,
    pub files: Vec<ManifestFile>,
    pub shared: Vec<SharedRecord>,
}

impl ManifestSession {
    pub fn bytes(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Manifest {
    pub version: u32,
    pub op_id: String,
    pub created_at: String,
    pub status: OpStatus,
    pub mode: CleanupMode,
    pub sessions: Vec<ManifestSession>,
    /// 공유 파일 백업 경로 (롤백/복구용).
    #[serde(default)]
    pub shared_backup: Option<String>,
}

impl Manifest {
    pub fn new(op_id: String, mode: CleanupMode) -> Manifest {
        Manifest {
            version: MANIFEST_VERSION,
            op_id,
            created_at: chrono::Local::now().to_rfc3339(),
            status: OpStatus::Pending,
            mode,
            sessions: Vec::new(),
            shared_backup: None,
        }
    }

    pub fn total_bytes(&self) -> u64 {
        self.sessions.iter().map(|s| s.bytes()).sum()
    }

    pub fn total_files(&self) -> usize {
        self.sessions.iter().map(|s| s.files.len()).sum()
    }

    pub fn save(&self, dir: &Path) -> anyhow::Result<()> {
        std::fs::create_dir_all(dir)?;
        let bytes = serde_json::to_vec_pretty(self)?;
        fsutil::atomic_write(&dir.join(MANIFEST_FILE), &bytes)?;
        Ok(())
    }

    pub fn load(dir: &Path) -> anyhow::Result<Manifest> {
        let text = std::fs::read_to_string(dir.join(MANIFEST_FILE))?;
        Ok(serde_json::from_str(&text)?)
    }

    pub fn op_dir(paths: &Paths, op_id: &str) -> PathBuf {
        paths.trash_dir().join(op_id)
    }

    /// 사람이 읽는 작업 시각 — 휴지통 화면에서 작업을 구분하는 기준.
    pub fn display_time(&self) -> String {
        chrono::DateTime::parse_from_rfc3339(&self.created_at)
            .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|_| self.op_id.clone())
    }
}

/// 작업 ID: 시각 + 프로세스 + 카운터. 같은 초에 두 번 실행해도 겹치지 않는다.
pub fn new_op_id() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!(
        "{}-{}{:03}",
        chrono::Local::now().format("%Y%m%d-%H%M%S"),
        std::process::id() % 1000,
        n % 1000
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Manifest {
        let mut m = Manifest::new("20260826-120000-001".into(), CleanupMode::Trash);
        m.sessions.push(ManifestSession {
            session_id: "aaaa".into(),
            project_key: "-w".into(),
            project_path: Some("/w".into()),
            display_name: "로그인 수정".into(),
            reasons: vec!["마지막 활동 후 92일 경과".into()],
            files: vec![
                ManifestFile {
                    original: "/c/projects/-w/aaaa.jsonl".into(),
                    stored: "files/0/aaaa.jsonl".into(),
                    size: 100,
                    is_dir: false,
                    moved_at: "2026-08-26T12:00:00+09:00".into(),
                },
                ManifestFile {
                    original: "/c/tasks/session-aaaa".into(),
                    stored: "files/0/session-aaaa".into(),
                    size: 20,
                    is_dir: true,
                    moved_at: "2026-08-26T12:00:00+09:00".into(),
                },
            ],
            shared: vec![SharedRecord {
                file: "/c/history.jsonl".into(),
                line_index: 3,
                content: "{}".into(),
            }],
        });
        m
    }

    #[test]
    fn round_trips_through_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("op");
        let m = sample();
        m.save(&dir).unwrap();
        assert_eq!(Manifest::load(&dir).unwrap(), m);
    }

    #[test]
    fn totals_are_summed_across_sessions() {
        let m = sample();
        assert_eq!(m.total_files(), 2);
        assert_eq!(m.total_bytes(), 120);
    }

    #[test]
    fn status_transition_is_persisted() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("op");
        let mut m = sample();
        m.save(&dir).unwrap();
        assert_eq!(Manifest::load(&dir).unwrap().status, OpStatus::Pending);
        m.status = OpStatus::Complete;
        m.save(&dir).unwrap();
        assert_eq!(Manifest::load(&dir).unwrap().status, OpStatus::Complete);
    }

    #[test]
    fn op_ids_are_unique_within_the_same_second() {
        let ids: std::collections::HashSet<String> = (0..50).map(|_| new_op_id()).collect();
        assert_eq!(ids.len(), 50);
    }

    #[test]
    fn loading_a_missing_manifest_is_an_error_not_a_panic() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(Manifest::load(tmp.path()).is_err());
    }
}
