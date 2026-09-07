//! 에이전트 데이터 스캔.
//!
//! PRD §15 성능: 2,000개 세션을 3초 안에. 세션 파일 분석을 rayon으로 병렬화하고,
//! 각 파일은 판정에 필요한 만큼만 읽는다(`jsonl::analyze`의 조기 중단).
//! 스캔은 백그라운드 스레드에서 돌고 TUI는 진행률을 받아 계속 그린다.

pub mod artifacts;
pub mod jsonl;
pub mod session;

use crate::agents::{Agent, SessionSeed};
use crate::paths::{Paths, decode_project_label};
use artifacts::{Artifact, PrefixIndex};
use jsonl::Analysis;
use rayon::prelude::*;
use session::{ORPHAN_KEY, Project, ScanResult, Session, SessionKind, UNKNOWN_PROJECT};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender};

pub enum ScanEvent {
    Progress { done: usize, total: usize },
    Done(Box<ScanResult>),
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
    let errors = Vec::new();

    // 설치된 에이전트만 훑는다. 없는 경로는 오류가 아니다 (PRD §14).
    let agents: Vec<Box<dyn Agent>> = crate::agents::registry()
        .into_iter()
        .filter(|a| a.present(paths))
        .collect();

    // 에이전트별로 씨앗과 고아 키를 모으고 전체 개수를 먼저 센다.
    let mut work: Vec<(Vec<SessionSeed>, Vec<String>, PrefixIndex)> = Vec::new();
    for agent in &agents {
        let seeds = agent.discover(paths);
        let known: HashSet<String> = seeds.iter().map(|s| s.id.clone()).collect();
        let orphans = agent.orphans(paths, &known);
        let ids: Vec<String> = seeds
            .iter()
            .map(|s| s.id.clone())
            .chain(orphans.iter().cloned())
            .collect();
        work.push((seeds, orphans, PrefixIndex::build(&ids)));
    }

    let total: usize = work.iter().map(|(s, o, _)| s.len() + o.len()).sum();
    let done = AtomicUsize::new(0);
    let bump = |done: &AtomicUsize| {
        let n = done.fetch_add(1, Ordering::Relaxed) + 1;
        on_progress(n, total);
    };

    let mut sessions: Vec<Session> = Vec::new();
    for (agent, (seeds, orphans, index)) in agents.iter().zip(work.iter()) {
        let built: Vec<Session> = seeds
            .par_iter()
            .map(|seed| {
                let s = build_session(paths, agent.as_ref(), seed, index);
                bump(&done);
                s
            })
            .collect();
        sessions.extend(built);

        let built_orphans: Vec<Session> = orphans
            .par_iter()
            .map(|key| {
                let s = build_orphan(paths, agent.as_ref(), key);
                bump(&done);
                s
            })
            .collect();
        sessions.extend(built_orphans);
    }

    ScanResult {
        projects: group(sessions),
        scanned_at_secs: now,
        errors,
    }
}

fn build_session(
    paths: &Paths,
    agent: &dyn Agent,
    seed: &SessionSeed,
    index: &PrefixIndex,
) -> Session {
    let analysis = agent.analyze(seed);
    let (arts, ambiguous) = agent.artifacts(paths, seed, index);

    let info = analysis.info();
    let cwd = info.and_then(|i| i.cwd.clone());
    let (project_path, project_exists) = verify_project(cwd);

    // 경로에서 알아낸 키를 우선하고, 없으면 cwd 를 프로젝트 키로 쓴다.
    let project_key = seed
        .project_key
        .clone()
        .or_else(|| {
            project_path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| UNKNOWN_PROJECT.to_string());

    let display_name = display_name_for(&seed.id, &analysis);
    let last_active_secs = last_active(&arts, info.and_then(|i| i.last_timestamp));
    let size_bytes = arts.iter().map(|a| a.size).sum();

    let kind = if seed.subagent || info.map(|i| i.is_sidechain).unwrap_or(false) {
        SessionKind::Subagent
    } else {
        SessionKind::Normal
    };

    Session {
        agent: agent.id(),
        id: seed.id.clone(),
        project_key,
        transcript: Some(seed.transcript.clone()),
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

fn build_orphan(paths: &Paths, agent: &dyn Agent, key: &str) -> Session {
    let arts = agent.orphan_artifacts(paths, key);
    let size_bytes = arts.iter().map(|a| a.size).sum();
    let last_active_secs = last_active(&arts, None);
    let short: String = key.chars().take(16).collect();
    Session {
        agent: agent.id(),
        id: key.to_string(),
        project_key: ORPHAN_KEY.to_string(),
        transcript: None,
        project_path: None,
        project_exists: None,
        display_name: format!("Leftover data {short}"),
        last_active_secs,
        size_bytes,
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
        Analysis::Unreadable(_) => format!("Unparseable {short}"),
        _ => analysis
            .info()
            .and_then(|i| {
                i.summary
                    .clone()
                    .or_else(|| i.first_prompt.clone())
                    .filter(|s| !s.trim().is_empty())
            })
            .unwrap_or_else(|| format!("Untitled {short}")),
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
        let idx = match projects
            .iter()
            .position(|p| p.key == s.project_key && p.agent == s.agent)
        {
            Some(i) => i,
            None => {
                let label = if s.project_key == ORPHAN_KEY {
                    "Orphaned data".to_string()
                } else if s.project_key == UNKNOWN_PROJECT {
                    "Unknown project".to_string()
                } else if s.project_key.starts_with('-') {
                    decode_project_label(&s.project_key)
                } else {
                    s.project_key.clone()
                };
                projects.push(Project {
                    agent: s.agent,
                    key: s.project_key.clone(),
                    label,
                    path: None,
                    exists: None,
                    sessions: Vec::new(),
                });
                projects.len() - 1
            }
        };
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
        p.sessions
            .sort_by_key(|s| std::cmp::Reverse(s.last_active_secs));
    }
    // 레지스트리 순 -> 고아는 뒤로 -> 이름순.
    let order: Vec<&'static str> = crate::agents::registry().iter().map(|a| a.id()).collect();
    projects.sort_by(|a, b| {
        let ai = order
            .iter()
            .position(|x| *x == a.agent)
            .unwrap_or(usize::MAX);
        let bi = order
            .iter()
            .position(|x| *x == b.agent)
            .unwrap_or(usize::MAX);
        let ao = (a.key == ORPHAN_KEY) as u8;
        let bo = (b.key == ORPHAN_KEY) as u8;
        ai.cmp(&bi)
            .then(ao.cmp(&bo))
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
