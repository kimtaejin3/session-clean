//! Claude Code. sclean 이 처음부터 지원한 에이전트다.
//!
//! ```text
//! ~/.claude/projects/<인코딩된 cwd>/<uuid>.jsonl   대화 기록
//! ~/.claude/projects/<인코딩된 cwd>/<uuid>/        하위 에이전트
//! ~/.claude/{tasks,teams,session-env,file-history,todos,debug}/
//! ~/.claude/sessions/<pid>.json                    실행 중 잠금
//! ~/.claude/history.jsonl                          공유 기록
//! ```
//!
//! 연결 데이터가 일곱 곳에 흩어져 있고 일부는 UUID 앞 8자로만 연결돼
//! 지원 대상 중 구조가 가장 복잡하다.

use super::{Agent, SessionSeed};
use crate::live::LiveSessions;
use crate::ops::fsutil;
use crate::paths::Paths;
use crate::scan::artifacts::{self, Artifact, PrefixIndex, looks_like_uuid};
use crate::scan::jsonl::{self, Analysis};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub struct ClaudeCode;

impl Agent for ClaudeCode {
    fn id(&self) -> &'static str {
        "claude"
    }

    fn label(&self) -> &'static str {
        "Claude Code"
    }

    fn root(&self, paths: &Paths) -> PathBuf {
        paths.claude_dir()
    }

    fn verified(&self) -> bool {
        true
    }

    fn discover(&self, paths: &Paths) -> Vec<SessionSeed> {
        let root = paths.projects_dir();
        if !root.is_dir() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for dir in fsutil::list_dir(&root) {
            if !dir.is_dir() {
                continue;
            }
            let key = artifacts::file_name_of(&dir);
            collect(&dir, &key, false, &mut out);
            // 세션 폴더 안의 subagents/ 도 세션으로 취급한다 (R4).
            for child in fsutil::list_dir(&dir) {
                if child.is_dir() {
                    let sub = child.join("subagents");
                    if sub.is_dir() {
                        collect(&sub, &key, true, &mut out);
                    }
                }
            }
        }
        out
    }

    fn analyze(&self, seed: &SessionSeed) -> Analysis {
        jsonl::analyze_with(&seed.transcript, &jsonl::absorb_claude)
    }

    fn artifacts(
        &self,
        paths: &Paths,
        seed: &SessionSeed,
        index: &PrefixIndex,
    ) -> (Vec<Artifact>, bool) {
        artifacts::collect_for(paths, &seed.id, Some(&seed.transcript), index)
    }

    fn orphans(&self, paths: &Paths, known: &HashSet<String>) -> Vec<String> {
        artifacts::orphan_session_ids(paths, known)
    }

    fn orphan_artifacts(&self, paths: &Paths, key: &str) -> Vec<Artifact> {
        artifacts::collect_orphan(paths, key)
    }

    fn live(&self, paths: &Paths) -> LiveSessions {
        LiveSessions::detect(paths)
    }

    fn shared_history(&self, paths: &Paths) -> Option<PathBuf> {
        let p = paths.history_file();
        p.is_file().then_some(p)
    }
}

fn collect(dir: &Path, key: &str, subagent: bool, out: &mut Vec<SessionSeed>) {
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
        let mut seed = SessionSeed::new(stem, p);
        seed.project_key = Some(key.to_string());
        seed.subagent = subagent;
        out.push(seed);
    }
}
