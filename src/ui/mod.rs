//! TUI 진입점과 이벤트 루프.
//!
//! 스캔은 백그라운드 스레드에서 돌고, 루프는 100ms 주기로 폴링하며 계속 그린다.
//! 그래서 2,000개 세션을 스캔하는 동안에도 화면이 멈춘 것처럼 보이지 않는다(PRD §15).

pub mod app;
pub mod filters;
pub mod modals;
pub mod sessions;
pub mod theme;
pub mod trash;

use crate::logging;
use crate::ops::manifest::CleanupMode;
use crate::paths::Paths;
use crate::scan::{ScanEvent, spawn_scan};
use app::{App, Focus, Screen};
use ratatui::crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
};
use ratatui::crossterm::{execute, terminal};
use ratatui::prelude::*;
use std::io::{Stdout, stdout};
use std::time::Duration;

const TICK: Duration = Duration::from_millis(100);

/// 터미널을 원상 복구하는 가드. 패닉이 나도 Drop이 실행된다.
struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> std::io::Result<TerminalGuard> {
        terminal::enable_raw_mode()?;
        execute!(stdout(), terminal::EnterAlternateScreen)?;
        Ok(TerminalGuard)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = execute!(stdout(), terminal::LeaveAlternateScreen);
    }
}

pub fn run(paths: Paths) -> anyhow::Result<()> {
    logging::init(&paths);
    let _guard = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal: Terminal<CrosstermBackend<Stdout>> = Terminal::new(backend)?;
    terminal.clear()?;

    let mut app = App::new(paths.clone());
    let rx = spawn_scan(paths);

    loop {
        // 스캔 진행 상황을 모두 흡수한다.
        while let Ok(ev) = rx.try_recv() {
            let done = matches!(ev, ScanEvent::Done(_));
            app.on_scan_event(ev);
            if done {
                break;
            }
        }

        terminal.draw(|frame| draw(frame, &app))?;

        if event::poll(TICK)?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            handle_key(&mut app, key);
        }
        if app.quit {
            break;
        }
    }
    Ok(())
}

fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    if area.width < theme::MIN_WIDTH || area.height < theme::MIN_HEIGHT {
        modals::render_too_small(frame, area);
        return;
    }
    let rows = Layout::vertical([Constraint::Min(3), Constraint::Length(2)]).split(area);

    match app.screen {
        Screen::Trash | Screen::TrashDetail => {
            trash::render(frame, app, area);
            if app.screen == Screen::TrashDetail {
                trash::render_detail(frame, app, area);
            }
            return;
        }
        _ => {}
    }

    sessions::render(frame, app, rows[0]);
    sessions::render_footer(frame, app, rows[1]);

    match app.screen {
        Screen::Confirm => modals::render_confirm(frame, app, area),
        Screen::Result => modals::render_result(frame, app, area),
        Screen::Filters => filters::render(frame, app, area),
        Screen::Help => modals::render_help(frame, area),
        Screen::Recovery => modals::render_recovery(frame, app, area),
        _ => {}
    }
}

pub fn handle_key(app: &mut App, key: KeyEvent) {
    // 검색 입력 중에는 문자를 그대로 받는다.
    if app.searching && app.screen == Screen::Sessions {
        match key.code {
            KeyCode::Esc => {
                app.clear_search();
                return;
            }
            KeyCode::Enter => {
                app.searching = false;
                return;
            }
            KeyCode::Backspace => {
                app.pop_search();
                return;
            }
            KeyCode::Char(c) => {
                app.push_search(c);
                return;
            }
            _ => {}
        }
    }

    match app.screen {
        Screen::Sessions => sessions_key(app, key),
        Screen::Confirm => confirm_key(app, key),
        Screen::Result => result_key(app, key),
        Screen::Trash => trash_key(app, key),
        Screen::TrashDetail => {
            if matches!(key.code, KeyCode::Esc | KeyCode::Enter) {
                app.screen = Screen::Trash;
            }
        }
        Screen::Filters => filters_key(app, key),
        Screen::Help => {
            if matches!(key.code, KeyCode::Esc | KeyCode::Char('?') | KeyCode::Enter) {
                app.screen = app.previous_screen;
            }
        }
        Screen::Recovery => match key.code {
            KeyCode::Char('r') | KeyCode::Char('R') => app.recover_pending(),
            KeyCode::Esc => app.skip_recovery(),
            KeyCode::Char('q') | KeyCode::Char('Q') => app.quit = true,
            _ => {}
        },
    }
}

fn sessions_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('q') | KeyCode::Char('Q') => app.quit = true,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => app.quit = true,
        KeyCode::Up | KeyCode::Char('k') => app.move_cursor(-1),
        KeyCode::Down | KeyCode::Char('j') => app.move_cursor(1),
        KeyCode::PageUp => app.move_cursor(-10),
        KeyCode::PageDown => app.move_cursor(10),
        KeyCode::Home => app.cursor = 0,
        KeyCode::End => app.move_cursor(isize::MAX / 2),
        KeyCode::Left => {
            if app.focus == Focus::Sessions && matches!(app.current_row(), Some(r) if r.session_id().is_some())
            {
                app.focus = Focus::Projects;
            } else {
                app.collapse_current();
            }
        }
        KeyCode::Right => {
            if app.focus == Focus::Projects {
                app.focus = Focus::Sessions;
            } else {
                app.expand_current();
            }
        }
        KeyCode::Char(' ') => app.toggle_current(),
        KeyCode::Char('a') | KeyCode::Char('A') => app.toggle_all_recommended(),
        KeyCode::Char('/') => app.start_search(),
        KeyCode::Char('d') | KeyCode::Char('D') => app.open_confirm(),
        KeyCode::Char('t') | KeyCode::Char('T') => app.open_trash(),
        KeyCode::Char('f') | KeyCode::Char('F') => {
            app.previous_screen = Screen::Sessions;
            app.screen = Screen::Filters;
        }
        KeyCode::Char('?') => {
            app.previous_screen = Screen::Sessions;
            app.screen = Screen::Help;
        }
        KeyCode::Esc => {
            if !app.search.is_empty() {
                app.clear_search();
            }
        }
        _ => {}
    }
}

fn confirm_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.screen = Screen::Sessions;
            app.status = "정리를 취소했습니다".into();
        }
        KeyCode::Char('t') | KeyCode::Char('T') if app.confirm.mode == CleanupMode::Permanent => {
            app.set_mode(CleanupMode::Trash)
        }
        KeyCode::Char('p') | KeyCode::Char('P') if app.confirm.mode == CleanupMode::Trash => {
            app.set_mode(CleanupMode::Permanent)
        }
        KeyCode::Enter => app.run_cleanup(),
        KeyCode::Backspace => {
            app.confirm.typed.pop();
        }
        // 완전 삭제 확인 낱말 입력.
        KeyCode::Char(c) if app.confirm.mode == CleanupMode::Permanent => {
            app.confirm.typed.push(c);
        }
        _ => {}
    }
}

fn result_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('l') | KeyCode::Char('L') => app.show_log = !app.show_log,
        KeyCode::Char('t') | KeyCode::Char('T') => app.open_trash(),
        KeyCode::Esc | KeyCode::Enter => {
            app.show_log = false;
            app.screen = Screen::Sessions;
        }
        KeyCode::Char('q') | KeyCode::Char('Q') => app.quit = true,
        _ => {}
    }
}

fn trash_key(app: &mut App, key: KeyEvent) {
    let last = app.trash_rows().len().saturating_sub(1);
    match key.code {
        KeyCode::Esc | KeyCode::Char('t') | KeyCode::Char('T') => app.screen = Screen::Sessions,
        KeyCode::Char('q') | KeyCode::Char('Q') => app.quit = true,
        KeyCode::Up | KeyCode::Char('k') => app.trash_cursor = app.trash_cursor.saturating_sub(1),
        KeyCode::Down | KeyCode::Char('j') => app.trash_cursor = (app.trash_cursor + 1).min(last),
        KeyCode::Char(' ') => app.toggle_trash_current(),
        KeyCode::Char('r') | KeyCode::Char('R') => app.restore_selection(),
        KeyCode::Char('x') | KeyCode::Char('X') => app.purge_selection(),
        KeyCode::Enter => app.screen = Screen::TrashDetail,
        KeyCode::Char('?') => {
            app.previous_screen = Screen::Trash;
            app.screen = Screen::Help;
        }
        _ => {}
    }
}

fn filters_key(app: &mut App, key: KeyEvent) {
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    match key.code {
        KeyCode::Esc | KeyCode::Char('f') | KeyCode::Char('F') => app.screen = Screen::Sessions,
        KeyCode::Char('q') | KeyCode::Char('Q') => app.quit = true,
        KeyCode::Up | KeyCode::Char('k') => {
            app.filter_cursor = app.filter_cursor.saturating_sub(1)
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.filter_cursor = (app.filter_cursor + 1).min(App::FILTER_ROWS - 1)
        }
        KeyCode::Char(' ') | KeyCode::Enter => app.toggle_filter_row(),
        KeyCode::Left if app.filter_cursor == 0 => app.adjust_threshold(if shift { -7 } else { -1 }),
        KeyCode::Right if app.filter_cursor == 0 => app.adjust_threshold(if shift { 7 } else { 1 }),
        KeyCode::Char('?') => {
            app.previous_screen = Screen::Filters;
            app.screen = Screen::Help;
        }
        _ => {}
    }
}
