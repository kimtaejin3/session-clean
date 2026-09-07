//! 코딩 에이전트 어댑터.
//!
//! sclean 의 안전 모델은 "세션 파일을 휴지통으로 옮겼다가 되돌린다"이다.
//! 이 전제가 성립하는 에이전트, 즉 **세션 하나가 파일 하나**인 것만 지원한다.
//! SQLite 에 세션을 넣는 도구(Cursor, OpenCode, Goose 등)는 행을 옮길 수 없어
//! 트랜잭션 설계가 달라지므로 범위 밖이다.
//!
//! 어댑터가 하는 일은 세 가지다.
//! 1. 세션 파일 찾기(`discover`)
//! 2. 그 파일에서 판정에 필요한 정보 뽑기(`analyze`)
//! 3. 함께 지워야 할 연결 파일 모으기(`artifacts`)
//!
//! 나머지(규칙 판정, 트랜잭션, 휴지통, 복원)는 전부 공통이다.

pub mod claude;
pub mod codex;
pub mod continue_dev;
pub mod copilot;
pub mod gemini;

use crate::live::LiveSessions;
use crate::paths::Paths;
use crate::scan::artifacts::{Artifact, PrefixIndex};
use crate::scan::jsonl::Analysis;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// 발견 단계에서 찾아낸 세션 후보. 아직 파일을 열어보지 않은 상태다.
#[derive(Clone, Debug)]
pub struct SessionSeed {
    pub id: String,
    pub transcript: PathBuf,
    /// 경로만으로 프로젝트를 알 수 있으면 채운다. 모르면 분석 후 `cwd` 로 정한다.
    pub project_key: Option<String>,
    /// 하위 에이전트 기록인가 (R4).
    pub subagent: bool,
}

impl SessionSeed {
    pub fn new(id: impl Into<String>, transcript: PathBuf) -> SessionSeed {
        SessionSeed {
            id: id.into(),
            transcript,
            project_key: None,
            subagent: false,
        }
    }
}

pub trait Agent: Send + Sync {
    /// 저장·표시에 쓰는 안정된 식별자.
    fn id(&self) -> &'static str;
    fn label(&self) -> &'static str;
    /// 이 에이전트의 데이터 루트. 정리 대상은 반드시 이 안에 있어야 한다.
    fn root(&self, paths: &Paths) -> PathBuf;

    /// 실제 데이터로 기록 형식을 확인했는가.
    ///
    /// 확인하지 못한 형식은 파싱에 실패해도 조용히 넘어가지 않고
    /// `분석 불가`로 표시되어 정리가 차단된다. 그래서 미검증 어댑터를
    /// 넣어도 안전하지만, 사용자에게는 사실대로 알린다.
    fn verified(&self) -> bool;

    fn present(&self, paths: &Paths) -> bool {
        self.root(paths).is_dir()
    }

    fn discover(&self, paths: &Paths) -> Vec<SessionSeed>;
    fn analyze(&self, seed: &SessionSeed) -> Analysis;

    /// 이 세션이 소유한 파일들과, 소유가 모호한지 여부.
    fn artifacts(
        &self,
        paths: &Paths,
        seed: &SessionSeed,
        index: &PrefixIndex,
    ) -> (Vec<Artifact>, bool) {
        let _ = (paths, index);
        let mut out = Vec::new();
        if let Some(a) = Artifact::at(
            seed.transcript.clone(),
            crate::scan::artifacts::ArtifactKind::Transcript,
        ) {
            out.push(a);
        }
        (out, false)
    }

    /// R5. 대화 기록 없이 남은 데이터. 기본은 없음.
    fn orphans(&self, paths: &Paths, known: &HashSet<String>) -> Vec<String> {
        let _ = (paths, known);
        Vec::new()
    }

    fn orphan_artifacts(&self, paths: &Paths, key: &str) -> Vec<Artifact> {
        let _ = (paths, key);
        Vec::new()
    }

    /// 실행 중인 세션. 감지할 수 없으면 빈 집합.
    fn live(&self, paths: &Paths) -> LiveSessions {
        let _ = paths;
        LiveSessions::empty()
    }

    /// 여러 세션이 공유해 통째로 옮길 수 없는 기록 파일.
    fn shared_history(&self, paths: &Paths) -> Option<PathBuf> {
        let _ = paths;
        None
    }
}

/// 지원하는 에이전트 목록. 순서가 화면 순서다.
pub fn registry() -> Vec<Box<dyn Agent>> {
    vec![
        Box::new(claude::ClaudeCode),
        Box::new(codex::Codex),
        Box::new(gemini::GeminiCli),
        Box::new(copilot::CopilotCli),
        Box::new(continue_dev::Continue),
    ]
}

pub fn by_id(id: &str) -> Option<Box<dyn Agent>> {
    registry().into_iter().find(|a| a.id() == id)
}

pub fn label_of(id: &str) -> String {
    by_id(id)
        .map(|a| a.label().to_string())
        .unwrap_or_else(|| id.to_string())
}

/// 이 에이전트의 기록 형식을 실제 데이터로 확인했는가.
pub fn verified_id(id: &str) -> bool {
    by_id(id).map(|a| a.verified()).unwrap_or(false)
}

/// 이 세션의 정리 대상이 놓여도 되는 루트.
pub fn root_of(paths: &Paths, id: &str) -> Option<PathBuf> {
    by_id(id).map(|a| a.root(paths))
}

/// 파일 이름을 세션 ID 로 쓰는 흔한 형태를 위한 공통 발견 함수.
pub fn discover_flat(dir: &Path, ext: &str) -> Vec<SessionSeed> {
    crate::ops::fsutil::list_dir(dir)
        .into_iter()
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some(ext))
        .filter_map(|p| {
            let id = p.file_stem()?.to_string_lossy().into_owned();
            Some(SessionSeed::new(id, p))
        })
        .collect()
}

/// 하위 디렉터리를 재귀로 훑어 확장자가 맞는 파일을 모은다.
pub fn discover_recursive(dir: &Path, ext: &str, out: &mut Vec<PathBuf>, depth: usize) {
    if depth == 0 {
        return;
    }
    for p in crate::ops::fsutil::list_dir(dir) {
        if p.is_dir() {
            discover_recursive(&p, ext, out, depth - 1);
        } else if p.extension().and_then(|e| e.to_str()) == Some(ext) {
            out.push(p);
        }
    }
}
