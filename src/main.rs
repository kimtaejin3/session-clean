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
            "-V" | "--version" => return println!("session-clean {}", env!("CARGO_PKG_VERSION")),
            other => {
                eprintln!("unknown argument: {other}");
                eprintln!("Run `session-clean --help` for usage.");
                std::process::exit(2);
            }
        }
    }

    let paths = match Paths::discover() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("could not resolve paths: {e:#}");
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
        eprintln!("error: {e:#}");
        eprintln!("log: {}", paths.log_file().display());
        std::process::exit(1);
    }
}

fn print_help() {
    println!(
        "session-clean {} — clean up the sessions your coding agents leave behind

USAGE
  session-clean              open the terminal UI
  session-clean --help       this help
  session-clean --version    version

  `sclean` also works, as a shorter alias.

KEYS
  \u{2191} \u{2193} move   \u{2192} sessions   \u{2190} projects   Space select
  A suggested   D clean   T trash   F rules   ? help   Q quit

AGENTS
  Claude Code, Codex, Gemini CLI, Copilot CLI, Continue
  Only agents that store one session per file are supported.

ENVIRONMENT
  SCLEAN_HOME         where agent data lives (default: your home directory)
  SCLEAN_CLAUDE_DIR   Claude Code data only (default: ~/.claude)
  SCLEAN_DATA_DIR     sclean's own storage
                      (default: ~/Library/Application Support/sclean, ~/.local/share/sclean on Linux)

No network access. Everything stays on this machine.",
        env!("CARGO_PKG_VERSION")
    );
}
