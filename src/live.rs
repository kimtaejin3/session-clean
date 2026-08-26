//! 실행 중인 Claude Code 세션 감지.
//!
//! `~/.claude/sessions/<pid>.json`이 실행 중 세션의 잠금 역할을 한다.
//! 파일 이름의 pid가 아직 살아있는지 `kill(pid, 0)`으로 확인하고, 살아있는
//! 프로세스의 세션 ID만 "실행 중"으로 본다. 죽은 프로세스의 잔여 파일은 무시한다.
//!
//! PRD §9: "현재 실행 중인 것으로 확인됨"은 정리를 차단하는 조건이다.
//! 감지에 실패하는 것(디렉터리 없음, 형식 변경)은 오류가 아니다 — 확신할 수
//! 없으면 차단하지 않되, 최근 활동 여부(`rules::Caution::RecentlyActive`)가
//! 두 번째 방어선이 된다.

use crate::ops::fsutil;
use crate::paths::Paths;
use crate::scan::artifacts::looks_like_uuid;
use serde_json::Value;
use std::collections::HashSet;

#[derive(Debug, Default, Clone)]
pub struct LiveSessions {
    ids: HashSet<String>,
    /// 잠금 디렉터리를 실제로 읽을 수 있었는가.
    pub detected: bool,
}

impl LiveSessions {
    pub fn empty() -> LiveSessions {
        LiveSessions::default()
    }

    pub fn detect(paths: &Paths) -> LiveSessions {
        let dir = paths.sessions_dir();
        if !dir.is_dir() {
            return LiveSessions::default();
        }
        let mut ids = HashSet::new();
        for p in fsutil::list_dir(&dir) {
            if p.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let stem = p
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            // `<pid>.json` 또는 `<pid>.<hash>.json`
            let Some(pid) = stem.split('.').next().and_then(|s| s.parse::<i32>().ok()) else {
                continue;
            };
            if !process_alive(pid) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&p) else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            collect_uuids(&value, &mut ids);
        }
        LiveSessions {
            ids,
            detected: true,
        }
    }

    pub fn contains(&self, session_id: &str) -> bool {
        self.ids.contains(session_id)
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
}

/// 잠금 파일의 형식이 바뀌어도 동작하도록, UUID처럼 생긴 문자열을 모두 모은다.
/// 과잉 수집은 안전한 방향으로만 실패한다(정리를 덜 하게 된다).
fn collect_uuids(v: &Value, out: &mut HashSet<String>) {
    match v {
        Value::String(s) => {
            if looks_like_uuid(s) {
                out.insert(s.clone());
            }
        }
        Value::Array(items) => items.iter().for_each(|i| collect_uuids(i, out)),
        Value::Object(map) => map.values().for_each(|i| collect_uuids(i, out)),
        _ => {}
    }
}

fn process_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    // signal 0은 아무것도 보내지 않고 존재/권한만 확인한다.
    unsafe { libc::kill(pid, 0) == 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIVE: &str = "aaaaaaaa-1111-2222-3333-444444444444";
    const DEAD: &str = "bbbbbbbb-1111-2222-3333-444444444444";

    #[test]
    fn detects_sessions_of_live_pids_only() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::with_roots(tmp.path().join("claude"), tmp.path().join("data"));
        let dir = paths.sessions_dir();
        std::fs::create_dir_all(&dir).unwrap();

        let mine = std::process::id();
        std::fs::write(
            dir.join(format!("{mine}.json")),
            format!(r#"{{"sessionId":"{LIVE}"}}"#),
        )
        .unwrap();
        // 살아있을 가능성이 거의 없는 pid.
        std::fs::write(
            dir.join("2147480000.json"),
            format!(r#"{{"sessionId":"{DEAD}"}}"#),
        )
        .unwrap();

        let live = LiveSessions::detect(&paths);
        assert!(live.detected);
        assert!(live.contains(LIVE));
        assert!(!live.contains(DEAD), "죽은 pid의 잠금은 무시한다");
    }

    #[test]
    fn missing_sessions_dir_is_not_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::with_roots(tmp.path().join("claude"), tmp.path().join("data"));
        let live = LiveSessions::detect(&paths);
        assert!(!live.detected);
        assert!(live.is_empty());
    }

    #[test]
    fn unknown_lock_format_yields_nothing_rather_than_panicking() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::with_roots(tmp.path().join("claude"), tmp.path().join("data"));
        std::fs::create_dir_all(paths.sessions_dir()).unwrap();
        let mine = std::process::id();
        std::fs::write(paths.sessions_dir().join(format!("{mine}.json")), b"nope").unwrap();
        assert!(LiveSessions::detect(&paths).is_empty());
    }

    #[test]
    fn finds_session_ids_nested_anywhere_in_the_lock_file() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::with_roots(tmp.path().join("claude"), tmp.path().join("data"));
        std::fs::create_dir_all(paths.sessions_dir()).unwrap();
        let mine = std::process::id();
        std::fs::write(
            paths.sessions_dir().join(format!("{mine}.abc.json")),
            format!(r#"{{"meta":{{"active":[{{"id":"{LIVE}"}}]}}}}"#),
        )
        .unwrap();
        assert!(LiveSessions::detect(&paths).contains(LIVE));
    }
}
