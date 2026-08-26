//! 로컬 파일 로거.
//!
//! PRD §15 개인정보 보호: **프롬프트 본문을 절대 기록하지 않는다.**
//! 호출자는 세션 ID, 경로, 크기, 규칙 이름, 오류 메시지만 넘긴다.
//! 로그 쓰기 실패는 무시한다 — 로그 때문에 정리가 실패하면 안 된다.

use crate::paths::Paths;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::OnceLock;

static LOG_PATH: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

fn slot() -> &'static Mutex<Option<PathBuf>> {
    LOG_PATH.get_or_init(|| Mutex::new(None))
}

pub fn init(paths: &Paths) {
    let _ = paths.ensure_data_dirs();
    if let Ok(mut guard) = slot().lock() {
        *guard = Some(paths.log_file());
    }
}

pub fn info(msg: &str) {
    write_line("INFO", msg);
}

pub fn error(msg: &str) {
    write_line("ERROR", msg);
}

fn write_line(level: &str, msg: &str) {
    let Ok(guard) = slot().lock() else { return };
    let Some(path) = guard.as_ref() else { return };
    let stamp = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%:z");
    // 개행은 로그 한 줄 구조를 깨뜨리므로 제거한다.
    let flat = msg.replace(['\n', '\r'], " ");
    let line = format!("{stamp} {level} {flat}\n");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = f.write_all(line.as_bytes());
    }
}

/// FR-18: TUI에서 오류 상세와 최근 로그를 볼 수 있게 한다.
pub fn tail(paths: &Paths, lines: usize) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(paths.log_file()) else {
        return Vec::new();
    };
    let all: Vec<&str> = text.lines().collect();
    all.iter()
        .rev()
        .take(lines)
        .rev()
        .map(|s| s.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_and_tails_lines_without_panicking_on_missing_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let p = Paths::with_roots(tmp.path().join("c"), tmp.path().join("d"));
        init(&p);
        info("scan complete sessions=3");
        error("move failed path=/x/y");
        let out = tail(&p, 10);
        assert_eq!(out.len(), 2);
        assert!(out[0].contains("INFO scan complete sessions=3"));
        assert!(out[1].contains("ERROR move failed"));
    }

    #[test]
    fn newlines_are_flattened_into_one_record() {
        let tmp = tempfile::tempdir().unwrap();
        let p = Paths::with_roots(tmp.path().join("c"), tmp.path().join("d"));
        init(&p);
        info("a\nb");
        assert_eq!(tail(&p, 10).len(), 1);
    }

    #[test]
    fn tail_of_missing_log_is_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let p = Paths::with_roots(tmp.path().join("c"), tmp.path().join("d"));
        assert!(tail(&p, 5).is_empty());
    }
}
