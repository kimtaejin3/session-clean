//! Gemini CLI.
//!
//! ```text
//! ~/.gemini/tmp/<project_hash>/chats/*.json
//! ```
//!
//! **형식을 실제 데이터로 확인하지 못했다**(이 맥에 대화 기록이 없다).
//! 문서 기준으로 작성했고, 실제 형식이 다르면 `분석 불가`로 표시되어
//! 정리가 차단된다 — 잘못 지우는 대신 아무것도 하지 않는다.
//!
//! 프로젝트가 경로 해시로만 표현되어 원래 디렉터리를 복원할 수 없다.
//! 따라서 기록 안에서 경로를 찾지 못하면 프로젝트 존재 여부(R2)를 적용하지 않는다.

use super::{Agent, SessionSeed};
use crate::ops::fsutil;
use crate::paths::Paths;
use crate::scan::jsonl::{Analysis, ParsedInfo, clip, describe_io_error, parse_timestamp};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub struct GeminiCli;

impl Agent for GeminiCli {
    fn id(&self) -> &'static str {
        "gemini"
    }

    fn label(&self) -> &'static str {
        "Gemini CLI"
    }

    fn root(&self, paths: &Paths) -> PathBuf {
        paths.agent_root("gemini")
    }

    fn verified(&self) -> bool {
        false
    }

    fn discover(&self, paths: &Paths) -> Vec<SessionSeed> {
        let mut out = Vec::new();
        // tmp/<project_hash>/chats/*.json
        for hash_dir in fsutil::list_dir(&self.root(paths).join("tmp")) {
            if !hash_dir.is_dir() {
                continue;
            }
            let key = hash_dir
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            for f in fsutil::list_dir(&hash_dir.join("chats")) {
                if f.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                let Some(stem) = f.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
                    continue;
                };
                let mut seed = SessionSeed::new(format!("{key}/{stem}"), f);
                seed.project_key = Some(key.clone());
                out.push(seed);
            }
        }
        out
    }

    fn analyze(&self, seed: &SessionSeed) -> Analysis {
        analyze_chat_json(&seed.transcript)
    }
}

fn analyze_chat_json(path: &Path) -> Analysis {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => return Analysis::Unreadable(describe_io_error(&e)),
    };
    let Ok(v) = serde_json::from_str::<Value>(&text) else {
        return Analysis::Unreadable("JSON 을 이해할 수 없습니다".into());
    };

    let mut info = ParsedInfo::default();

    // 저장 형식이 버전에 따라 다르므로 대화 배열이 있을 만한 자리를 모두 본다.
    let turns = ["messages", "history", "turns", "chat"]
        .iter()
        .find_map(|k| v.get(*k).and_then(Value::as_array));

    let Some(turns) = turns else {
        return Analysis::Unreadable("대화 기록을 찾지 못했습니다".into());
    };

    for turn in turns {
        if let Some(ts) = turn
            .get("timestamp")
            .or_else(|| turn.get("createdAt"))
            .and_then(Value::as_str)
            && let Some(secs) = parse_timestamp(ts)
        {
            crate::scan::jsonl::note_timestamp(&mut info, secs);
        }

        let role = turn
            .get("role")
            .or_else(|| turn.get("type"))
            .and_then(Value::as_str);
        let text = turn
            .get("parts")
            .and_then(parts_text)
            .or_else(|| turn.get("content").and_then(parts_text))
            .or_else(|| turn.get("text").and_then(Value::as_str).map(str::to_string));

        match role {
            Some("user") => {
                crate::scan::jsonl::note_user_turn(&mut info, &text.unwrap_or_default())
            }
            Some("model") | Some("assistant") => {
                // 도구 호출은 parts 안의 functionCall 로 온다.
                if let Some(parts) = turn.get("parts").and_then(Value::as_array) {
                    info.tool_uses += parts
                        .iter()
                        .filter(|p| p.get("functionCall").is_some())
                        .count();
                }
            }
            _ => {}
        }
    }

    if let Some(t) = v.get("title").and_then(Value::as_str)
        && !t.trim().is_empty()
    {
        info.summary = Some(clip(t));
    }

    Analysis::Parsed(info)
}

fn parts_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Array(parts) => {
            let mut out = String::new();
            for p in parts {
                if let Some(t) = p.get("text").and_then(Value::as_str).or_else(|| p.as_str()) {
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
    fn reads_the_documented_shape() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("session-1.json");
        std::fs::write(
            &p,
            r#"{"title":"버그 추적","messages":[
                {"role":"user","parts":[{"text":"왜 안 되지"}]},
                {"role":"model","parts":[{"functionCall":{"name":"read_file"}}]},
                {"role":"user","parts":[{"text":"다시"}]}]}"#,
        )
        .unwrap();
        let a = analyze_chat_json(&p);
        let i = a.info().unwrap();
        assert_eq!(i.user_messages, 2);
        assert_eq!(i.tool_uses, 1);
        assert_eq!(i.summary.as_deref(), Some("버그 추적"));
    }

    #[test]
    fn an_unexpected_shape_is_unreadable_so_cleanup_is_blocked() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("session-2.json");
        std::fs::write(&p, r#"{"somethingElse": 1}"#).unwrap();
        assert!(
            analyze_chat_json(&p).is_unreadable(),
            "형식을 모르면 지우지 않는다"
        );
    }

    #[test]
    fn discovery_keys_sessions_by_project_hash() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::with_home(tmp.path().to_path_buf(), tmp.path().join("data"));
        let chats = paths.agent_root("gemini").join("tmp/abc123/chats");
        std::fs::create_dir_all(&chats).unwrap();
        std::fs::write(chats.join("session-1.json"), "{}").unwrap();

        let seeds = GeminiCli.discover(&paths);
        assert_eq!(seeds.len(), 1);
        assert_eq!(seeds[0].project_key.as_deref(), Some("abc123"));
        assert_eq!(seeds[0].id, "abc123/session-1");
    }
}
