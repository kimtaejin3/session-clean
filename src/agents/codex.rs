//! OpenAI Codex CLI.
//!
//! 이 맥의 실제 데이터로 형식을 확인했다(2026-09-07, 세션 97개 · 162 MB).
//!
//! ```text
//! ~/.codex/sessions/YYYY/MM/DD/rollout-<ISO>-<uuid>.jsonl
//! ~/.codex/archived_sessions/rollout-*.jsonl
//! ~/.codex/history.jsonl                      공유 기록
//! ```
//!
//! 각 줄은 `{timestamp, type, ordinal, payload}` 이고 `type` 은 다음 중 하나다.
//!
//! | type | 뜻 |
//! |---|---|
//! | `session_meta` | 첫 줄. `payload.cwd` 와 `payload.session_id` 가 있다 |
//! | `response_item` | `payload.type` 이 `message`(role 포함) / `custom_tool_call` / `reasoning` |
//! | `event_msg` | 진행 상황. 판정에 쓰지 않는다 |
//! | `turn_context` | `cwd` 를 다시 담고 있다 |
//!
//! Claude 와 달리 `cwd` 가 **첫 줄에 확정적으로** 들어 있어 프로젝트 판정이 더 확실하다.

use super::{Agent, SessionSeed, discover_recursive};
use crate::paths::Paths;
use crate::scan::jsonl::{self, Analysis, ParsedInfo};
use serde_json::Value;
use std::path::PathBuf;

pub struct Codex;

impl Agent for Codex {
    fn id(&self) -> &'static str {
        "codex"
    }

    fn label(&self) -> &'static str {
        "Codex"
    }

    fn root(&self, paths: &Paths) -> PathBuf {
        paths.agent_root("codex")
    }

    fn verified(&self) -> bool {
        true
    }

    fn discover(&self, paths: &Paths) -> Vec<SessionSeed> {
        let root = self.root(paths);
        let mut files = Vec::new();
        // sessions/ 는 YYYY/MM/DD 로 세 단계 내려간다.
        discover_recursive(&root.join("sessions"), "jsonl", &mut files, 5);
        discover_recursive(&root.join("archived_sessions"), "jsonl", &mut files, 3);

        files
            .into_iter()
            .filter_map(|p| {
                let stem = p.file_stem()?.to_string_lossy().into_owned();
                // rollout-<ISO 시각>-<uuid> 에서 뒤쪽 uuid 를 세션 ID 로 쓴다.
                let id = session_id_from_stem(&stem)?;
                Some(SessionSeed::new(id, p))
            })
            .collect()
    }

    fn analyze(&self, seed: &SessionSeed) -> Analysis {
        jsonl::analyze_with(&seed.transcript, &absorb)
    }

    fn shared_history(&self, paths: &Paths) -> Option<PathBuf> {
        let p = self.root(paths).join("history.jsonl");
        p.is_file().then_some(p)
    }
}

/// `rollout-2026-09-04T20-16-25-01a06c22-57aa-77e1-b335-c1d0a455ea69`
/// 에서 끝의 UUID 를 떼어낸다. 형태가 다르면 전체 stem 을 쓴다.
pub fn session_id_from_stem(stem: &str) -> Option<String> {
    if stem.is_empty() {
        return None;
    }
    let parts: Vec<&str> = stem.split('-').collect();
    if parts.len() >= 5 {
        let tail = parts[parts.len() - 5..].join("-");
        if crate::scan::artifacts::looks_like_uuid(&tail) {
            return Some(tail);
        }
    }
    Some(stem.to_string())
}

fn absorb(info: &mut ParsedInfo, v: &Value) {
    if let Some(ts) = v.get("timestamp").and_then(Value::as_str)
        && let Some(secs) = jsonl::parse_timestamp(ts)
    {
        jsonl::note_timestamp(info, secs);
    }

    let Some(payload) = v.get("payload") else {
        return;
    };

    match v.get("type").and_then(Value::as_str) {
        // 첫 줄. cwd 와 세션 ID 가 확정적으로 들어 있다.
        Some("session_meta") | Some("turn_context") => {
            if info.cwd.is_none()
                && let Some(cwd) = payload.get("cwd").and_then(Value::as_str)
                && !cwd.is_empty()
            {
                info.cwd = Some(PathBuf::from(cwd));
            }
        }
        Some("response_item") => match payload.get("type").and_then(Value::as_str) {
            Some("message") => {
                if payload.get("role").and_then(Value::as_str) == Some("user")
                    && let Some(text) = text_of(payload.get("content"))
                {
                    jsonl::note_user_turn(info, &text);
                }
            }
            // Codex 는 도구 호출을 별도 항목으로 남긴다.
            Some("custom_tool_call") | Some("function_call") | Some("local_shell_call") => {
                info.tool_uses += 1;
            }
            _ => {}
        },
        _ => {}
    }
}

/// content 는 문자열이거나 `{type, text}` 블록 배열이다.
fn text_of(content: Option<&Value>) -> Option<String> {
    match content {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Array(blocks)) => {
            let mut text = String::new();
            for b in blocks {
                if let Some(t) = b.get("text").and_then(Value::as_str) {
                    if !text.is_empty() {
                        text.push(' ');
                    }
                    text.push_str(t);
                }
            }
            Some(text)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_session_id_from_rollout_name() {
        assert_eq!(
            session_id_from_stem(
                "rollout-2026-09-04T20-16-25-01a06c22-57aa-77e1-b335-c1d0a455ea69"
            )
            .unwrap(),
            "01a06c22-57aa-77e1-b335-c1d0a455ea69"
        );
    }

    #[test]
    fn falls_back_to_the_whole_stem_for_unknown_names() {
        assert_eq!(session_id_from_stem("weird-name").unwrap(), "weird-name");
    }

    #[test]
    fn reads_cwd_user_turns_and_tool_calls() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("rollout-x.jsonl");
        std::fs::write(
            &f,
            [
                r#"{"timestamp":"2026-09-04T20:16:25Z","type":"session_meta","ordinal":0,"payload":{"cwd":"/w/shop","session_id":"01a06c22-57aa-77e1-b335-c1d0a455ea69"}}"#,
                r#"{"timestamp":"2026-09-04T20:16:30Z","type":"response_item","ordinal":1,"payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"로그인 고쳐줘"}]}}"#,
                r#"{"timestamp":"2026-09-04T20:16:40Z","type":"event_msg","ordinal":2,"payload":{"type":"task_started"}}"#,
                r#"{"timestamp":"2026-09-04T20:17:00Z","type":"response_item","ordinal":3,"payload":{"type":"custom_tool_call","name":"shell","status":"completed"}}"#,
            ]
            .join("\n"),
        )
        .unwrap();

        let seed = SessionSeed::new("01a06c22-57aa-77e1-b335-c1d0a455ea69", f);
        let a = Codex.analyze(&seed);
        let i = a.info().unwrap();
        assert_eq!(i.cwd, Some(PathBuf::from("/w/shop")));
        assert_eq!(i.user_messages, 1);
        assert_eq!(i.tool_uses, 1);
        assert_eq!(i.first_prompt.as_deref(), Some("로그인 고쳐줘"));
    }

    #[test]
    fn a_single_question_session_is_short() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("rollout-y.jsonl");
        std::fs::write(
            &f,
            [
                r#"{"timestamp":"2026-09-04T20:16:25Z","type":"session_meta","payload":{"cwd":"/w"}}"#,
                r#"{"timestamp":"2026-09-04T20:16:30Z","type":"response_item","payload":{"type":"message","role":"user","content":"한 번만"}}"#,
                r#"{"timestamp":"2026-09-04T20:16:35Z","type":"response_item","payload":{"type":"message","role":"assistant","content":"네"}}"#,
            ]
            .join("\n"),
        )
        .unwrap();
        let a = Codex.analyze(&SessionSeed::new("y", f));
        assert!(a.is_usable());
        let i = a.info().unwrap();
        assert_eq!(i.user_messages, 1, "assistant 는 세지 않는다");
        assert_eq!(i.tool_uses, 0);
    }

    #[test]
    fn unknown_format_is_unreadable_not_a_panic() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("rollout-z.jsonl");
        std::fs::write(&f, "이건 JSON이 아니다\n<html>").unwrap();
        assert!(Codex.analyze(&SessionSeed::new("z", f)).is_unreadable());
    }
}
