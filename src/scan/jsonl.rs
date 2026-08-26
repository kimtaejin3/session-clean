//! Claude 세션 JSONL의 관대한 스트리밍 분석기.
//!
//! PRD §15 호환성: "Claude 내부 JSONL 형식이 바뀌어도 앱이 종료되지 않고
//! 해당 세션을 `분석 불가`로 처리해야 한다." 따라서 강타입 역직렬화를 쓰지 않고
//! `serde_json::Value`를 줄 단위로 읽으며, 필요한 필드가 없으면 그냥 없는 것으로 둔다.
//!
//! PRD §15 성능: "세션 전체 내용을 메모리에 계속 보관하지 않는다."
//! R3(짧은 세션) 판정이 확정되는 순간 읽기를 멈춘다. 실제 세션은 대부분
//! 처음 몇 줄만 읽고 끝난다.

use serde_json::Value;
use std::io::BufRead;
use std::path::{Path, PathBuf};

/// 한 세션에서 읽어들일 최대 바이트. 이 이상은 `Partial`로 처리한다.
pub const MAX_SCAN_BYTES: u64 = 4 * 1024 * 1024;
/// 표시용 첫 프롬프트 최대 길이(문자).
pub const DISPLAY_LEN: usize = 120;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParsedInfo {
    /// 도구 결과와 메타 메시지를 제외한 실제 사용자 턴 수.
    pub user_messages: usize,
    pub tool_uses: usize,
    pub first_prompt: Option<String>,
    pub summary: Option<String>,
    pub cwd: Option<PathBuf>,
    pub is_sidechain: bool,
    pub last_timestamp: Option<i64>,
    pub broken_lines: usize,
    /// 조기 중단했거나 바이트 상한에 걸려 끝까지 읽지 않았다.
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Analysis {
    /// 파일 전체를 문제없이 이해했다(조기 중단 포함 — 판정에 필요한 만큼은 읽었다).
    Parsed(ParsedInfo),
    /// 일부 줄이 깨졌다. 읽은 정보는 표시하되 추천에는 쓰지 않는다.
    Partial(ParsedInfo),
    /// 열 수 없거나 한 줄도 이해하지 못했다.
    Unreadable(String),
}

impl Analysis {
    pub fn info(&self) -> Option<&ParsedInfo> {
        match self {
            Analysis::Parsed(i) | Analysis::Partial(i) => Some(i),
            Analysis::Unreadable(_) => None,
        }
    }

    /// 규칙 판정(R3/R4)에 쓸 수 있을 만큼 신뢰할 수 있는가.
    ///
    /// PRD §9: "JSONL을 정상적으로 분석한 경우에만 적용한다."
    pub fn is_usable(&self) -> bool {
        matches!(self, Analysis::Parsed(i) if !i.truncated_before_conclusion())
    }

    pub fn is_unreadable(&self) -> bool {
        matches!(self, Analysis::Unreadable(_))
    }
}

impl ParsedInfo {
    /// 상한에 걸려 R3 판정을 확정하지 못한 상태인가.
    ///
    /// 조기 중단은 "이미 짧은 세션이 아님이 확정된" 경우에만 일어나므로
    /// 판정을 방해하지 않는다. 반대로 바이트 상한(`hit_byte_limit`)에 걸린
    /// 경우는 아직 확정되지 않았다.
    fn truncated_before_conclusion(&self) -> bool {
        self.truncated && self.user_messages <= 1 && self.tool_uses == 0
    }
}

pub fn analyze(path: &Path) -> Analysis {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) => return Analysis::Unreadable(describe_io_error(&e)),
    };
    let mut reader = std::io::BufReader::with_capacity(64 * 1024, file);
    let mut info = ParsedInfo::default();
    let mut parsed_any = false;
    let mut read_bytes: u64 = 0;
    let mut line = String::new();

    loop {
        line.clear();
        let n = match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => {
                // 중간에 읽기가 깨져도 여기까지 얻은 정보는 살린다.
                if !parsed_any {
                    return Analysis::Unreadable(describe_io_error(&e));
                }
                info.broken_lines += 1;
                break;
            }
        };
        read_bytes += n as u64;

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
            info.broken_lines += 1;
            continue;
        };
        parsed_any = true;
        absorb(&mut info, &value);

        if can_stop(&info) {
            info.truncated = true;
            break;
        }
        if read_bytes >= MAX_SCAN_BYTES {
            info.truncated = true;
            break;
        }
    }

    if !parsed_any {
        return Analysis::Unreadable("이해할 수 있는 JSONL 줄이 없습니다".into());
    }
    if info.broken_lines > 0 {
        Analysis::Partial(info)
    } else {
        Analysis::Parsed(info)
    }
}

/// R3 판정이 확정됐고 표시에 필요한 정보도 다 모였으면 더 읽지 않는다.
fn can_stop(info: &ParsedInfo) -> bool {
    let short_rule_settled = info.user_messages > 1 || info.tool_uses > 0;
    let display_settled = info.summary.is_some() || info.first_prompt.is_some();
    short_rule_settled && display_settled && info.cwd.is_some()
}

fn absorb(info: &mut ParsedInfo, v: &Value) {
    if info.cwd.is_none()
        && let Some(cwd) = v.get("cwd").and_then(Value::as_str)
        && !cwd.is_empty()
    {
        info.cwd = Some(PathBuf::from(cwd));
    }
    if v.get("isSidechain").and_then(Value::as_bool) == Some(true) {
        info.is_sidechain = true;
    }
    if let Some(ts) = v.get("timestamp").and_then(Value::as_str)
        && let Some(secs) = parse_timestamp(ts)
    {
        info.last_timestamp = Some(info.last_timestamp.map_or(secs, |p| p.max(secs)));
    }

    match v.get("type").and_then(Value::as_str) {
        Some("summary") => {
            if info.summary.is_none()
                && let Some(s) = v.get("summary").and_then(Value::as_str)
            {
                info.summary = Some(clip(s));
            }
        }
        Some("user") => {
            if v.get("isMeta").and_then(Value::as_bool) == Some(true) {
                return;
            }
            let content = v.get("message").and_then(|m| m.get("content"));
            if let Some(text) = user_text(content) {
                info.user_messages += 1;
                if info.first_prompt.is_none() && !text.is_empty() {
                    info.first_prompt = Some(clip(&text));
                }
            }
        }
        Some("assistant") => {
            if let Some(arr) = v
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(Value::as_array)
            {
                info.tool_uses += arr
                    .iter()
                    .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_use"))
                    .count();
            }
        }
        _ => {}
    }
}

/// 사용자 턴의 표시용 텍스트. 도구 결과만 담긴 줄은 사용자 턴이 아니다.
///
/// Claude Code는 도구 실행 결과도 `type: "user"` 줄로 기록한다. 그것까지
/// 세면 R3(짧은 세션)가 거의 맞지 않게 되므로 반드시 걸러야 한다.
fn user_text(content: Option<&Value>) -> Option<String> {
    match content {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Array(blocks)) => {
            let mut has_real_block = false;
            let mut text = String::new();
            for b in blocks {
                match b.get("type").and_then(Value::as_str) {
                    Some("tool_result") => {}
                    Some("text") => {
                        has_real_block = true;
                        if let Some(t) = b.get("text").and_then(Value::as_str) {
                            if !text.is_empty() {
                                text.push(' ');
                            }
                            text.push_str(t);
                        }
                    }
                    // image, document 등도 사용자가 보낸 턴이다.
                    Some(_) => has_real_block = true,
                    None => {}
                }
            }
            has_real_block.then_some(text)
        }
        _ => None,
    }
}

/// 표시용으로 개행을 없애고 길이를 자른다. 로그에는 절대 넣지 않는다(PRD §15).
fn clip(s: &str) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= DISPLAY_LEN {
        flat
    } else {
        let cut: String = flat.chars().take(DISPLAY_LEN).collect();
        format!("{cut}…")
    }
}

fn parse_timestamp(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp())
}

fn describe_io_error(e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::PermissionDenied => "읽기 권한이 없습니다".into(),
        std::io::ErrorKind::NotFound => "파일이 없습니다".into(),
        _ => format!("읽기 실패: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(lines: &[&str]) -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("s.jsonl");
        std::fs::write(&p, lines.join("\n")).unwrap();
        (tmp, p)
    }

    #[test]
    fn counts_user_turns_ignoring_tool_results_and_meta() {
        // 도구 사용 줄을 넣지 않아 조기 중단 없이 필터링 자체를 검증한다.
        let (_t, p) = write(&[
            r#"{"type":"user","cwd":"/w","message":{"role":"user","content":"첫 질문"}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"ok"}]}}"#,
            r#"{"type":"user","isMeta":true,"message":{"content":"메타"}}"#,
            r#"{"type":"user","message":{"content":"두번째 질문"}}"#,
        ]);
        let a = analyze(&p);
        let i = a.info().unwrap();
        assert_eq!(i.user_messages, 2, "도구 결과와 메타는 세지 않는다");
        assert_eq!(i.tool_uses, 0);
        assert_eq!(i.first_prompt.as_deref(), Some("첫 질문"));
        assert_eq!(i.cwd, Some(PathBuf::from("/w")));
    }

    #[test]
    fn a_tool_result_only_session_is_still_short() {
        // 사용자 턴 1개 + 도구 결과 줄만 있는 세션은 R3 대상이어야 한다.
        let (_t, p) = write(&[
            r#"{"type":"user","cwd":"/w","message":{"content":"한 번만"}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"x"}]}}"#,
        ]);
        let i = analyze(&p).info().unwrap().clone();
        assert_eq!(i.user_messages, 1);
        assert_eq!(i.tool_uses, 0);
    }

    #[test]
    fn image_only_user_turn_counts_as_a_real_turn() {
        let (_t, p) = write(&[
            r#"{"type":"user","cwd":"/w","message":{"content":"보기"}}"#,
            r#"{"type":"user","message":{"content":[{"type":"image","source":{}}]}}"#,
        ]);
        let i = analyze(&p).info().unwrap().clone();
        assert_eq!(i.user_messages, 2);
    }

    #[test]
    fn short_session_has_one_user_message_and_no_tools() {
        let (_t, p) = write(&[
            r#"{"type":"user","cwd":"/w","message":{"content":"안녕"}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"네"}]}}"#,
        ]);
        let a = analyze(&p);
        assert!(a.is_usable());
        let i = a.info().unwrap();
        assert_eq!(i.user_messages, 1);
        assert_eq!(i.tool_uses, 0);
    }

    #[test]
    fn survives_broken_lines_and_reports_partial() {
        let (_t, p) = write(&[
            r#"{"type":"user","cwd":"/w","message":{"content":"질문"}}"#,
            "이건 JSON이 아니다",
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read"}]}}"#,
        ]);
        let a = analyze(&p);
        assert!(matches!(a, Analysis::Partial(_)));
        assert!(!a.is_usable(), "손상된 세션은 추천 근거로 쓰지 않는다");
        let i = a.info().unwrap();
        assert_eq!(i.broken_lines, 1);
        assert_eq!(i.first_prompt.as_deref(), Some("질문"));
    }

    #[test]
    fn unknown_schema_is_unreadable_not_panic() {
        let (_t, p) = write(&["not json at all", "<html></html>"]);
        let a = analyze(&p);
        assert!(a.is_unreadable());
        assert!(a.info().is_none());
    }

    #[test]
    fn missing_file_is_unreadable() {
        assert!(analyze(Path::new("/nope/nope.jsonl")).is_unreadable());
    }

    #[test]
    fn detects_sidechain_and_summary() {
        let (_t, p) = write(&[
            r#"{"type":"summary","summary":"로그인 수정"}"#,
            r#"{"type":"user","cwd":"/w","isSidechain":true,"message":{"content":"하위 작업"}}"#,
        ]);
        let i = analyze(&p).info().unwrap().clone();
        assert!(i.is_sidechain);
        assert_eq!(i.summary.as_deref(), Some("로그인 수정"));
    }

    #[test]
    fn tracks_latest_timestamp() {
        let (_t, p) = write(&[
            r#"{"type":"user","cwd":"/w","timestamp":"2026-01-01T00:00:00Z","message":{"content":"a"}}"#,
            r#"{"type":"user","timestamp":"2026-03-01T00:00:00Z","message":{"content":"b"}}"#,
        ]);
        let i = analyze(&p).info().unwrap().clone();
        assert_eq!(i.last_timestamp, Some(1772323200));
    }

    #[test]
    fn stops_early_once_short_rule_is_settled() {
        // 판정에 필요한 정보가 다 모인 뒤의 줄은 읽지 않는다.
        let mut lines = vec![
            r#"{"type":"user","cwd":"/w","message":{"content":"q1"}}"#.to_string(),
            r#"{"type":"user","message":{"content":"q2"}}"#.to_string(),
        ];
        for i in 0..500 {
            lines.push(format!(
                r#"{{"type":"user","message":{{"content":"filler {i}"}}}}"#
            ));
        }
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("s.jsonl");
        std::fs::write(&p, lines.join("\n")).unwrap();
        let i = analyze(&p).info().unwrap().clone();
        assert_eq!(i.user_messages, 2, "조기 중단해야 한다");
        assert!(i.truncated);
        assert!(analyze(&p).is_usable(), "조기 중단은 판정을 막지 않는다");
    }

    #[test]
    fn long_first_prompt_is_clipped_for_display() {
        let long = "가".repeat(400);
        let line = format!(r#"{{"type":"user","cwd":"/w","message":{{"content":"{long}"}}}}"#);
        let (_t, p) = write(&[&line]);
        let i = analyze(&p).info().unwrap().clone();
        let shown = i.first_prompt.unwrap();
        assert_eq!(shown.chars().count(), DISPLAY_LEN + 1);
        assert!(shown.ends_with('…'));
    }

    #[test]
    fn empty_file_is_unreadable() {
        let (_t, p) = write(&[]);
        assert!(analyze(&p).is_unreadable());
    }
}
