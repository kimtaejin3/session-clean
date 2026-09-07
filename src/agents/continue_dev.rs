//! Continue (VS Code / JetBrains 확장의 CLI 저장소).
//!
//! 이 맥의 실제 데이터로 형식을 확인했다(2026-09-07).
//!
//! ```text
//! ~/.continue/sessions/<uuid>.json
//! ```
//!
//! 파일 하나가 세션 하나이고 최상위에 필요한 것이 다 들어 있다.
//! `title` 은 표시 이름으로, `workspaceDirectory` 는 프로젝트 경로로 그대로 쓴다.

use super::{Agent, SessionSeed, discover_flat};
use crate::paths::Paths;
use crate::scan::jsonl::{Analysis, ParsedInfo, clip, describe_io_error, parse_timestamp};
use serde_json::Value;
use std::path::PathBuf;

pub struct Continue;

impl Agent for Continue {
    fn id(&self) -> &'static str {
        "continue"
    }

    fn label(&self) -> &'static str {
        "Continue"
    }

    fn root(&self, paths: &Paths) -> PathBuf {
        paths.agent_root("continue")
    }

    fn verified(&self) -> bool {
        true
    }

    fn discover(&self, paths: &Paths) -> Vec<SessionSeed> {
        discover_flat(&self.root(paths).join("sessions"), "json")
    }

    fn analyze(&self, seed: &SessionSeed) -> Analysis {
        analyze_session_json(&seed.transcript)
    }
}

fn analyze_session_json(path: &std::path::Path) -> Analysis {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => return Analysis::Unreadable(describe_io_error(&e)),
    };
    let Ok(v) = serde_json::from_str::<Value>(&text) else {
        return Analysis::Unreadable("could not parse the JSON".into());
    };

    let mut info = ParsedInfo::default();

    if let Some(w) = v.get("workspaceDirectory").and_then(Value::as_str)
        && !w.is_empty()
    {
        info.cwd = Some(PathBuf::from(w));
    }
    if let Some(t) = v.get("title").and_then(Value::as_str)
        && !t.trim().is_empty()
    {
        info.summary = Some(clip(t));
    }

    let Some(history) = v.get("history").and_then(Value::as_array) else {
        // 대화가 없는 파일도 형식은 이해한 것이다.
        return Analysis::Parsed(info);
    };

    for entry in history {
        let Some(message) = entry.get("message") else {
            continue;
        };
        if let Some(ts) = message
            .get("createdAt")
            .or_else(|| entry.get("createdAt"))
            .and_then(Value::as_str)
            && let Some(secs) = parse_timestamp(ts)
        {
            crate::scan::jsonl::note_timestamp(&mut info, secs);
        }

        match message.get("role").and_then(Value::as_str) {
            Some("user") => {
                let text = message
                    .get("content")
                    .and_then(content_text)
                    .unwrap_or_default();
                crate::scan::jsonl::note_user_turn(&mut info, &text);
            }
            // 도구 호출은 assistant 메시지의 toolCalls 로 남는다.
            Some("assistant") => {
                if let Some(calls) = message.get("toolCalls").and_then(Value::as_array) {
                    info.tool_uses += calls.len();
                }
            }
            _ => {}
        }
        // contextItems 는 사용자가 붙인 파일이므로 도구 실행이 아니다.
    }

    Analysis::Parsed(info)
}

fn content_text(v: &Value) -> Option<String> {
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

    fn write(body: &str) -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("s.json");
        std::fs::write(&p, body).unwrap();
        (tmp, p)
    }

    #[test]
    fn reads_title_workspace_and_turns() {
        let (_t, p) = write(
            r#"{
              "sessionId": "82b61a95",
              "title": "로그인 리팩터링",
              "workspaceDirectory": "/w/shop",
              "mode": "agent",
              "history": [
                {"message": {"role": "user", "content": "고쳐줘"}},
                {"message": {"role": "assistant", "content": "네", "toolCalls": [{"id": "1"}]}},
                {"message": {"role": "user", "content": "하나 더"}}
              ]
            }"#,
        );
        let a = analyze_session_json(&p);
        let i = a.info().unwrap();
        assert_eq!(i.summary.as_deref(), Some("로그인 리팩터링"));
        assert_eq!(i.cwd, Some(PathBuf::from("/w/shop")));
        assert_eq!(i.user_messages, 2);
        assert_eq!(i.tool_uses, 1);
    }

    #[test]
    fn a_one_question_session_is_short() {
        let (_t, p) = write(
            r#"{"title":"질문","workspaceDirectory":"/w","history":[
                {"message":{"role":"user","content":"이게 뭐야"}},
                {"message":{"role":"assistant","content":"설명"}}]}"#,
        );
        let a = analyze_session_json(&p);
        assert!(a.is_usable());
        assert_eq!(a.info().unwrap().user_messages, 1);
        assert_eq!(a.info().unwrap().tool_uses, 0);
    }

    #[test]
    fn broken_json_is_unreadable() {
        let (_t, p) = write("{ 깨진");
        assert!(analyze_session_json(&p).is_unreadable());
    }

    #[test]
    fn a_file_without_history_is_still_understood() {
        let (_t, p) = write(r#"{"title":"빈 세션","workspaceDirectory":"/w"}"#);
        let a = analyze_session_json(&p);
        assert!(!a.is_unreadable());
        assert_eq!(a.info().unwrap().user_messages, 0);
    }
}
