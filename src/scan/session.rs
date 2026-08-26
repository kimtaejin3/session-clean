//! 스캔 결과 모델.

use crate::scan::artifacts::{Artifact, ArtifactKind};
use crate::scan::jsonl::Analysis;
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SessionKind {
    Normal,
    /// 하위 에이전트 세션 (R4).
    Subagent,
    /// 대화 기록 없이 연결 데이터만 남은 것 (R5).
    Orphan,
}

/// 고아 데이터를 모아 보여주는 가상 프로젝트 키.
pub const ORPHAN_KEY: &str = "__orphan__";

#[derive(Clone, Debug)]
pub struct Session {
    pub id: String,
    pub project_key: String,
    pub transcript: Option<PathBuf>,
    /// 세션 기록의 `cwd`로 확인한 실제 프로젝트 경로.
    pub project_path: Option<PathBuf>,
    /// `None`이면 신뢰할 수 있게 확인하지 못한 것 — R2를 적용하지 않는다.
    pub project_exists: Option<bool>,
    pub display_name: String,
    pub last_active_secs: i64,
    pub size_bytes: u64,
    pub analysis: Analysis,
    pub kind: SessionKind,
    pub artifacts: Vec<Artifact>,
    pub ambiguous_ownership: bool,
}

impl Session {
    pub fn file_count(&self) -> usize {
        self.artifacts.len()
    }

    pub fn artifact_kinds(&self) -> Vec<ArtifactKind> {
        let mut kinds: Vec<ArtifactKind> = Vec::new();
        for a in &self.artifacts {
            if !kinds.contains(&a.kind) {
                kinds.push(a.kind);
            }
        }
        kinds
    }

    /// 검색 대상 텍스트 (FR-04).
    pub fn matches(&self, needle_lower: &str) -> bool {
        self.display_name.to_lowercase().contains(needle_lower)
            || self.id.to_lowercase().contains(needle_lower)
    }
}

#[derive(Clone, Debug)]
pub struct Project {
    /// `projects/` 아래 디렉터리 이름 또는 `ORPHAN_KEY`.
    pub key: String,
    /// 표시용 전체 경로 문자열.
    pub label: String,
    /// `cwd`로 확인한 실제 경로 (확인 실패 시 `None`).
    pub path: Option<PathBuf>,
    pub exists: Option<bool>,
    pub sessions: Vec<Session>,
}

impl Project {
    pub fn short_label(&self) -> String {
        if self.key == ORPHAN_KEY {
            "고아 데이터".to_string()
        } else {
            crate::paths::short_label(&self.label)
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ScanResult {
    pub projects: Vec<Project>,
    pub scanned_at_secs: i64,
    /// 사용자에게 보여줄 비치명적 문제 (권한 부족 등).
    pub errors: Vec<String>,
}

impl ScanResult {
    pub fn session_count(&self) -> usize {
        self.projects.iter().map(|p| p.sessions.len()).sum()
    }

    pub fn find(&self, project_key: &str, session_id: &str) -> Option<&Session> {
        self.projects
            .iter()
            .find(|p| p.key == project_key)?
            .sessions
            .iter()
            .find(|s| s.id == session_id)
    }

    pub fn sessions(&self) -> impl Iterator<Item = &Session> {
        self.projects.iter().flat_map(|p| p.sessions.iter())
    }
}
