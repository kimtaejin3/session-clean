//! `sclean` — Claude Code 세션 정리 도구.
//!
//! 실행하면 곧바로 TUI가 뜬다. 옵션은 없다 (PRD §1: "사용자는 `sclean` 하나만 실행한다").
//! 테스트와 수동 검증을 위해 `SCLEAN_CLAUDE_DIR` / `SCLEAN_DATA_DIR` 환경변수로
//! 데이터 위치를 바꿀 수 있다.

use sclean::{logging, paths::Paths, ui};

fn main() {
    if let Some(arg) = std::env::args().nth(1) {
        match arg.as_str() {
            "-h" | "--help" => return print_help(),
            "-V" | "--version" => return println!("sclean {}", env!("CARGO_PKG_VERSION")),
            other => {
                eprintln!("알 수 없는 인자입니다: {other}");
                eprintln!("`sclean --help` 를 확인하세요.");
                std::process::exit(2);
            }
        }
    }

    let paths = match Paths::discover() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("경로를 확인하지 못했습니다: {e:#}");
            std::process::exit(1);
        }
    };

    // 패닉이 나도 터미널을 되살리고, 무슨 일이 있었는지 로그에 남긴다.
    // 패닉 메시지에는 프롬프트 본문이 들어가지 않는다 — 코어는 세션 본문을
    // 보관하지 않고 표시용으로 잘라낸 문자열만 들고 있기 때문이다.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = ratatui::crossterm::terminal::disable_raw_mode();
        let _ = ratatui::crossterm::execute!(
            std::io::stdout(),
            ratatui::crossterm::terminal::LeaveAlternateScreen
        );
        logging::error(&format!("panic: {info}"));
        default_hook(info);
    }));

    if let Err(e) = ui::run(paths.clone()) {
        logging::error(&format!("fatal: {e:#}"));
        eprintln!("오류: {e:#}");
        eprintln!("로그: {}", paths.log_file().display());
        std::process::exit(1);
    }
}

fn print_help() {
    println!(
        "sclean {} — Claude Code 세션 정리 도구

사용법:
  sclean              TUI를 엽니다
  sclean --help       이 도움말
  sclean --version    버전

조작:
  ↑ ↓ 이동   Space 선택   A 추천 전체   D 정리
  T 휴지통   F 추천 기준   / 검색   ? 도움말   Q 종료

환경변수:
  SCLEAN_CLAUDE_DIR   Claude 데이터 위치 (기본: ~/.claude)
  SCLEAN_DATA_DIR     sclean 저장소 (기본: ~/Library/Application Support/sclean)

네트워크를 사용하지 않으며 모든 데이터는 이 Mac에만 저장됩니다.",
        env!("CARGO_PKG_VERSION")
    );
}
