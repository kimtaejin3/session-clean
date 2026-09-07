//! GitHub Copilot CLI.
//!
//! ```text
//! ~/.copilot/session-state/          현재
//! ~/.copilot/history-session-state/  구버전
//! ```
//!
//! **형식을 실제 데이터로 확인하지 못했다.** 이 맥에 `~/.copilot` 은 있지만
//! 세션 기록이 없다. 문서 기준으로 작성했고, 실제 형식이 다르면
//! `분석 불가`로 표시되어 정리가 차단된다.

use super::{Agent, SessionSeed, discover_flat};
use crate::paths::Paths;
use crate::scan::jsonl::{self, Analysis, ParsedInfo};
use serde_json::Value;
use std::path::PathBuf;

pub struct CopilotCli;

/// 세션이 들어 있을 수 있는 디렉터리. 존재하는 것만 읽는다.
const SESSION_DIRS: &[&str] = &["session-state", "history-session-state"];

impl Agent for CopilotCli {
    fn id(&self) -> &'static str {
        "copilot"
    }

    fn label(&self) -> &'static str {
        "Copilot CLI"
    }

    fn root(&self, paths: &Paths) -> PathBuf {
        paths.agent_root("copilot")
    }

    fn verified(&self) -> bool {
        false
    }

    fn discover(&self, paths: &Paths) -> Vec<SessionSeed> {
        let root = self.root(paths);
        let mut out = Vec::new();
        for dir in SESSION_DIRS {
            let d = root.join(dir);
            out.extend(discover_flat(&d, "jsonl"));
            out.extend(discover_flat(&d, "json"));
        }
        out
    }

    fn analyze(&self, seed: &SessionSeed) -> Analysis {
        // 확장자에 따라 줄 단위인지 통 JSON 인지 나뉜다.
        if seed.transcript.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            jsonl::analyze_with(&seed.transcript, &absorb)
        } else {
            analyze_whole(&seed.transcript)
        }
    }
}

fn absorb(info: &mut ParsedInfo, v: &Value) {
    if let Some(ts) = v
        .get("timestamp")
        .or_else(|| v.get("createdAt"))
        .and_then(Value::as_str)
        && let Some(secs) = jsonl::parse_timestamp(ts)
    {
        jsonl::note_timestamp(info, secs);
    }
    if info.cwd.is_none()
        && let Some(cwd) = v
            .get("cwd")
            .or_else(|| v.get("workingDirectory"))
            .and_then(Value::as_str)
        && !cwd.is_empty()
    {
        info.cwd = Some(PathBuf::from(cwd));
    }
    absorb_turn(info, v);
}

/// 한 항목이 사용자 턴인지 도구 호출인지 판정한다.
fn absorb_turn(info: &mut ParsedInfo, v: &Value) {
    let role = v
        .get("role")
        .or_else(|| v.get("type"))
        .and_then(Value::as_str);
    match role {
        Some("user") => {
            let text = v
                .get("content")
                .and_then(text_of)
                .or_else(|| v.get("text").and_then(Value::as_str).map(str::to_string))
                .unwrap_or_default();
            jsonl::note_user_turn(info, &text);
        }
        Some("assistant") | Some("model") => {
            if let Some(calls) = v
                .get("toolCalls")
                .or_else(|| v.get("tool_calls"))
                .and_then(Value::as_array)
            {
                info.tool_uses += calls.len();
            }
        }
        Some("tool") | Some("tool_call") | Some("function_call") => info.tool_uses += 1,
        _ => {}
    }
}

fn analyze_whole(path: &std::path::Path) -> Analysis {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => return Analysis::Unreadable(jsonl::describe_io_error(&e)),
    };
    let Ok(v) = serde_json::from_str::<Value>(&text) else {
        return Analysis::Unreadable("JSON 을 이해할 수 없습니다".into());
    };
    let mut info = ParsedInfo::default();
    if let Some(cwd) = v
        .get("cwd")
        .or_else(|| v.get("workingDirectory"))
        .and_then(Value::as_str)
    {
        info.cwd = Some(PathBuf::from(cwd));
    }
    let turns = ["messages", "history", "turns", "events"]
        .iter()
        .find_map(|k| v.get(*k).and_then(Value::as_array));
    let Some(turns) = turns else {
        return Analysis::Unreadable("대화 기록을 찾지 못했습니다".into());
    };
    for t in turns {
        absorb_turn(&mut info, t);
    }
    Analysis::Parsed(info)
}

fn text_of(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Array(parts) => {
            let mut out = String::new();
            for p in parts {
                if let Some(t) = p.get("text").and_then(Value::as_str) {
                    if !out.is_empty() {
                        out.push(' ');
                    }
                    out.push_str(t);
                }
            }
            Some(out)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_line_oriented_session() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("s.jsonl");
        std::fs::write(
            &p,
            [
                r#"{"timestamp":"2026-09-01T10:00:00Z","role":"user","cwd":"/w","content":"고쳐줘"}"#,
                r#"{"role":"assistant","content":"네","toolCalls":[{"id":"1"},{"id":"2"}]}"#,
            ]
            .join("\n"),
        )
        .unwrap();
        let a = CopilotCli.analyze(&SessionSeed::new("s", p));
        let i = a.info().unwrap();
        assert_eq!(i.user_messages, 1);
        assert_eq!(i.tool_uses, 2);
        assert_eq!(i.cwd, Some(PathBuf::from("/w")));
    }

    #[test]
    fn reads_a_whole_file_session() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("s.json");
        std::fs::write(
            &p,
            r#"{"cwd":"/w","messages":[{"role":"user","content":"q1"},{"role":"user","content":"q2"}]}"#,
        )
        .unwrap();
        let a = CopilotCli.analyze(&SessionSeed::new("s", p));
        assert_eq!(a.info().unwrap().user_messages, 2);
    }

    #[test]
    fn an_unexpected_shape_blocks_cleanup() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("s.json");
        std::fs::write(&p, r#"{"nope":1}"#).unwrap();
        assert!(
            CopilotCli
                .analyze(&SessionSeed::new("s", p))
                .is_unreadable()
        );
    }
}
