//! Claude 데이터 스캔.
//!
//! PRD §15 성능: 2,000개 세션을 3초 안에. 세션 파일 분석을 rayon으로 병렬화하고,
//! 각 파일은 판정에 필요한 만큼만 읽는다(`jsonl::analyze`의 조기 중단).
//! 스캔은 백그라운드 스레드에서 돌고 TUI는 진행률을 받아 계속 그린다.

pub mod artifacts;
pub mod jsonl;
pub mod session;

use crate::ops::fsutil;
use crate::paths::{Paths, decode_project_label};
use artifacts::{Artifact, PrefixIndex, file_name_of, looks_like_uuid};
use jsonl::Analysis;
use rayon::prelude::*;
use session::{ORPHAN_KEY, Project, ScanResult, Session, SessionKind};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender};

pub enum ScanEvent {
    Progress { done: usize, total: usize },
    Done(Box<ScanResult>),
}

/// 발견 단계에서 찾아낸 트랜스크립트 후보.
struct Candidate {
    id: String,
    project_key: String,
    transcript: PathBuf,
    /// `projects/<p>/<uuid>/subagents/` 아래에서 나온 것인가.
    from_subagents: bool,
}

pub fn scan(paths: &Paths) -> ScanResult {
    scan_with_progress(paths, &|_, _| {})
}

/// TUI가 살아있도록 백그라운드에서 스캔한다.
pub fn spawn_scan(paths: Paths) -> Receiver<ScanEvent> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let tx2: Sender<ScanEvent> = tx.clone();
        let result = scan_with_progress(&paths, &move |done, total| {
            let _ = tx2.send(ScanEvent::Progress { done, total });
        });
        let _ = tx.send(ScanEvent::Done(Box::new(result)));
    });
    rx
}

pub fn scan_with_progress(
    paths: &Paths,
    on_progress: &(dyn Fn(usize, usize) + Sync),
) -> ScanResult {
    let now = now_secs();
    let mut errors = Vec::new();

    // PRD §14: Claude 데이터 디렉터리가 없어도 오류가 아니다.
    let candidates = discover_candidates(paths, &mut errors);
    let known: HashSet<String> = candidates.iter().map(|c| c.id.clone()).collect();
    let orphan_keys = artifacts::orphan_session_ids(paths, &known);

    let all_ids: Vec<String> = candidates
        .iter()
        .map(|c| c.id.clone())
        .chain(orphan_keys.iter().cloned())
        .collect();
    let index = Arc::new(PrefixIndex::build(&all_ids));

    let total = candidates.len() + orphan_keys.len();
    let done = AtomicUsize::new(0);
    let bump = |done: &AtomicUsize| {
        let n = done.fetch_add(1, Ordering::Relaxed) + 1;
        // 진행률 보고는 저렴해야 한다. 매 항목마다 보내되 채널이 흡수한다.
        on_progress(n, total);
    };

    let mut sessions: Vec<Session> = candidates
        .par_iter()
        .map(|c| {
            let s = build_session(paths, c, &index);
            bump(&done);
            s
        })
        .collect();

    let orphans: Vec<Session> = orphan_keys
        .par_iter()
        .map(|key| {
            let s = build_orphan(paths, key);
            bump(&done);
            s
        })
        .collect();
    sessions.extend(orphans);

    ScanResult {
        projects: group(sessions),
        scanned_at_secs: now,
        errors,
    }
}

/// `projects/` 아래를 훑어 세션 트랜스크립트를 모은다. 파일 내용은 읽지 않는다.
fn discover_candidates(paths: &Paths, errors: &mut Vec<String>) -> Vec<Candidate> {
    let root = paths.projects_dir();
    if !root.is_dir() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(&root) {
        Ok(e) => e,
        Err(e) => {
            errors.push(format!("{} 을(를) 읽을 수 없습니다: {e}", root.display()));
            return out;
        }
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let key = file_name_of(&dir);
        collect_transcripts(&dir, &key, false, &mut out);
        // 세션 폴더 안의 subagents/ 도 세션으로 취급한다 (R4).
        for child in fsutil::list_dir(&dir) {
            if child.is_dir() {
                let sub = child.join("subagents");
                if sub.is_dir() {
                    collect_transcripts(&sub, &key, true, &mut out);
                }
            }
        }
    }
    out
}

fn collect_transcripts(dir: &Path, key: &str, from_subagents: bool, out: &mut Vec<Candidate>) {
    for p in fsutil::list_dir(dir) {
        if p.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let stem = p
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        if !looks_like_uuid(&stem) {
            continue;
        }
        out.push(Candidate {
            id: stem,
            project_key: key.to_string(),
            transcript: p,
            from_subagents,
        });
    }
}

fn build_session(paths: &Paths, c: &Candidate, index: &PrefixIndex) -> Session {
    let analysis = jsonl::analyze(&c.transcript);
    let (arts, ambiguous) = artifacts::collect_for(paths, &c.id, Some(&c.transcript), index);

    let info = analysis.info();
    let cwd = info.and_then(|i| i.cwd.clone());
    let (project_path, project_exists) = verify_project(cwd);

    let display_name = display_name_for(&c.id, &analysis);
    let last_active_secs = last_active(&arts, info.and_then(|i| i.last_timestamp));
    let size_bytes = arts.iter().map(|a| a.size).sum();

    let kind = if c.from_subagents || info.map(|i| i.is_sidechain).unwrap_or(false) {
        SessionKind::Subagent
    } else {
        SessionKind::Normal
    };

    Session {
        id: c.id.clone(),
        project_key: c.project_key.clone(),
        transcript: Some(c.transcript.clone()),
        project_path,
        project_exists,
        display_name,
        last_active_secs,
        size_bytes,
        analysis,
        kind,
        artifacts: arts,
        ambiguous_ownership: ambiguous,
    }
}

fn build_orphan(paths: &Paths, key: &str) -> Session {
    let arts = artifacts::collect_orphan(paths, key);
    let size_bytes = arts.iter().map(|a| a.size).sum();
    let last_active_secs = last_active(&arts, None);
    let short: String = key.chars().take(16).collect();
    Session {
        id: key.to_string(),
        project_key: ORPHAN_KEY.to_string(),
        transcript: None,
        project_path: None,
        project_exists: None,
        display_name: format!("남은 데이터 {short}"),
        last_active_secs,
        size_bytes,
        // 대화 기록이 없으니 분석할 것도 없다 — 규칙은 R5만 적용된다.
        analysis: Analysis::Parsed(jsonl::ParsedInfo::default()),
        kind: SessionKind::Orphan,
        artifacts: arts,
        ambiguous_ownership: false,
    }
}

/// PRD §11.1: 프로젝트 경로는 `cwd`를 우선한다. 확인하지 못하면 존재 여부를
/// 판정하지 않는다(`None`) — 폴더 이름 디코딩은 손실이 있어 신뢰할 수 없다.
fn verify_project(cwd: Option<PathBuf>) -> (Option<PathBuf>, Option<bool>) {
    match cwd {
        Some(p) if p.is_absolute() => {
            // 존재 확인만 한다. 내용은 절대 읽지 않는다 (FR-16).
            let exists = std::fs::symlink_metadata(&p).is_ok();
            (Some(p), Some(exists))
        }
        other => (other, None),
    }
}

fn display_name_for(id: &str, analysis: &Analysis) -> String {
    let short: String = id.chars().take(8).collect();
    match analysis {
        Analysis::Unreadable(_) => format!("분석 불가 {short}"),
        _ => analysis
            .info()
            .and_then(|i| {
                i.summary
                    .clone()
                    .or_else(|| i.first_prompt.clone())
                    .filter(|s| !s.trim().is_empty())
            })
            .unwrap_or_else(|| format!("제목 없음 {short}")),
    }
}

fn last_active(arts: &[Artifact], timestamp: Option<i64>) -> i64 {
    let from_files = arts
        .iter()
        .map(|a| a.fingerprint.mtime_secs)
        .max()
        .unwrap_or(0);
    timestamp.map_or(from_files, |t| t.max(from_files))
}

fn group(sessions: Vec<Session>) -> Vec<Project> {
    let mut projects: Vec<Project> = Vec::new();
    for s in sessions {
        let idx = match projects.iter().position(|p| p.key == s.project_key) {
            Some(i) => i,
            None => {
                let label = if s.project_key == ORPHAN_KEY {
                    "고아 데이터".to_string()
                } else {
                    decode_project_label(&s.project_key)
                };
                projects.push(Project {
                    key: s.project_key.clone(),
                    label,
                    path: None,
                    exists: None,
                    sessions: Vec::new(),
                });
                projects.len() - 1
            }
        };
        // 프로젝트의 실제 경로는 이 프로젝트에 속한 세션 중 확인된 첫 값을 쓴다.
        if projects[idx].path.is_none()
            && let Some(p) = &s.project_path
        {
            projects[idx].label = p.to_string_lossy().into_owned();
            projects[idx].path = Some(p.clone());
            projects[idx].exists = s.project_exists;
        }
        projects[idx].sessions.push(s);
    }

    for p in &mut projects {
        // 최근 활동 순 — 오래된 것을 아래로 모아 훑기 쉽게 한다.
        p.sessions
            .sort_by_key(|s| std::cmp::Reverse(s.last_active_secs));
    }
    // 고아 데이터는 항상 마지막, 나머지는 이름순.
    projects.sort_by(|a, b| {
        let ao = (a.key == ORPHAN_KEY) as u8;
        let bo = (b.key == ORPHAN_KEY) as u8;
        ao.cmp(&bo)
            .then_with(|| a.short_label().cmp(&b.short_label()))
    });
    projects
}

pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
