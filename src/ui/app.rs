//! TUI 상태기계.
//!
//! 렌더링과 완전히 분리되어 있어 터미널 없이 테스트할 수 있다.
//! 키 입력 -> 상태 변화 -> (필요하면) 코어 연산 호출, 이 세 단계만 한다.

use crate::config::{Config, MAX_OLD_DAYS, MIN_OLD_DAYS};
use crate::live::LiveSessions;
use crate::logging;
use crate::ops::cleanup::{self, CleanupOutcome, CleanupPreview, CleanupTarget};
use crate::ops::manifest::CleanupMode;
use crate::ops::trash::{self, RestoreOutcome, TrashOp};
use crate::paths::Paths;
use crate::rules::{Verdict, evaluate};
use crate::scan::ScanEvent;
use crate::scan::now_secs;
use crate::scan::session::ORPHAN_KEY;
use crate::scan::session::ScanResult;
use std::collections::{HashMap, HashSet};

/// 완전 삭제를 확정하려면 정확히 이 낱말을 입력해야 한다 (PRD §8.2).
pub const DELETE_WORD: &str = "DELETE";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Screen {
    Sessions,
    Confirm,
    Result,
    Trash,
    TrashDetail,
    Filters,
    Help,
    Recovery,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Focus {
    Projects,
    Sessions,
}

#[derive(Clone, Debug, Default)]
pub struct ConfirmState {
    pub preview: CleanupPreview,
    pub mode: CleanupMode,
    pub typed: String,
}

impl ConfirmState {
    /// 완전 삭제는 `DELETE`를 정확히 입력해야만 실행할 수 있다.
    pub fn can_execute(&self) -> bool {
        if self.preview.sessions == 0 {
            return false;
        }
        match self.mode {
            CleanupMode::Trash => true,
            CleanupMode::Permanent => self.typed == DELETE_WORD,
        }
    }
}

#[derive(Clone, Debug)]
pub enum ScanState {
    Scanning { done: usize, total: usize },
    Ready,
}

pub struct App {
    pub paths: Paths,
    pub config: Config,
    pub scan_state: ScanState,
    pub result: ScanResult,
    pub verdicts: HashMap<String, Verdict>,
    pub live: LiveSessions,

    /// 왼쪽 패널 커서 — `visible_projects()` 안의 위치.
    pub project_cursor: usize,
    /// 오른쪽 패널 커서 — 현재 프로젝트의 `visible_sessions()` 안의 위치.
    pub session_cursor: usize,
    pub selected: HashSet<String>,

    pub screen: Screen,
    pub previous_screen: Screen,
    pub focus: Focus,

    pub confirm: ConfirmState,
    pub outcome: Option<CleanupOutcome>,
    pub restore_outcome: Option<RestoreOutcome>,

    pub trash_ops: Vec<TrashOp>,
    pub trash_cursor: usize,
    pub trash_selected: HashSet<(String, String)>,
    pub pending_ops: Vec<TrashOp>,

    pub status: String,
    pub show_log: bool,
    pub quit: bool,
    pub filter_cursor: usize,
    now: i64,
}

impl App {
    pub fn new(paths: Paths) -> App {
        let config = Config::load(&paths);
        let live = LiveSessions::detect(&paths);
        let pending_ops = trash::incomplete(&paths);
        // FR-19: 중단된 작업이 있으면 복구 화면을 최우선으로 띄운다.
        let screen = if pending_ops.is_empty() {
            Screen::Sessions
        } else {
            Screen::Recovery
        };
        App {
            trash_ops: trash::list(&paths),
            paths,
            config,
            scan_state: ScanState::Scanning { done: 0, total: 0 },
            result: ScanResult::default(),
            verdicts: HashMap::new(),
            live,
            project_cursor: 0,
            session_cursor: 0,
            selected: HashSet::new(),
            screen,
            previous_screen: Screen::Sessions,
            focus: Focus::Projects,
            confirm: ConfirmState::default(),
            outcome: None,
            restore_outcome: None,
            trash_cursor: 0,
            trash_selected: HashSet::new(),
            pending_ops,
            status: String::new(),
            show_log: false,
            quit: false,
            filter_cursor: 0,
            now: now_secs(),
        }
    }

    // ---------- 스캔 ----------

    pub fn on_scan_event(&mut self, ev: ScanEvent) {
        match ev {
            ScanEvent::Progress { done, total } => {
                self.scan_state = ScanState::Scanning { done, total };
            }
            ScanEvent::Done(result) => {
                self.result = *result;
                self.scan_state = ScanState::Ready;
                self.now = now_secs();
                self.recompute_verdicts();
                self.clamp_cursors();
                // PRD §6: 시작 시 추천 세션을 자동으로 선택하지 않는다.
                self.selected.clear();
                let n = self.result.session_count();
                let rec = self.recommended_ids().len();
                self.status = format!("세션 {n}개 · 추천 {rec}개 (아무것도 선택되지 않음)");
                logging::info(&format!("scan complete sessions={n} recommended={rec}"));
            }
        }
    }

    /// 파일을 바꾼 뒤 화면을 실제 상태와 다시 맞춘다.
    ///
    /// 이걸 빼먹으면 방금 지운 세션이 목록에 그대로 남는다.
    /// 스캔은 2,000개 기준 수십 ms라 그 자리에서 해도 된다.
    pub fn rescan(&mut self) {
        self.result = crate::scan::scan(&self.paths);
        self.now = now_secs();
        self.live = LiveSessions::detect(&self.paths);
        self.recompute_verdicts();
        // 사라진 세션은 선택에서도 빠져야 한다.
        let alive: std::collections::HashSet<String> =
            self.result.sessions().map(|s| s.id.clone()).collect();
        self.selected.retain(|id| alive.contains(id));
        self.clamp_cursors();
    }

    pub fn recompute_verdicts(&mut self) {
        let cfg = self.config.clone();
        let now = self.now;
        self.verdicts = self
            .result
            .sessions()
            .map(|s| (s.id.clone(), evaluate(s, &cfg, now, &self.live)))
            .collect();
        // 더 이상 정리할 수 없게 된 세션은 선택에서 뺀다.
        let blocked: Vec<String> = self
            .verdicts
            .iter()
            .filter(|(_, v)| !v.cleanable())
            .map(|(k, _)| k.clone())
            .collect();
        for id in blocked {
            self.selected.remove(&id);
        }
    }

    pub fn verdict(&self, id: &str) -> Option<&Verdict> {
        self.verdicts.get(id)
    }

    // ---------- 보이는 항목 ----------
    //
    // 왼쪽에서 고른 프로젝트의 세션만 오른쪽에 보인다. 두 패널은 각자
    // 커서를 갖고, `←` `→` 로 어느 쪽을 움직일지 정한다.

    /// 목록에 보이는 프로젝트들의 인덱스.
    pub fn visible_projects(&self) -> Vec<usize> {
        (0..self.result.projects.len()).collect()
    }

    pub fn current_project(&self) -> Option<&crate::scan::session::Project> {
        self.result.projects.get(self.project_cursor)
    }

    /// 현재 프로젝트의 세션들.
    pub fn visible_sessions(&self) -> Vec<&crate::scan::session::Session> {
        match self.current_project() {
            Some(p) => p.sessions.iter().collect(),
            None => Vec::new(),
        }
    }

    pub fn current_session(&self) -> Option<&crate::scan::session::Session> {
        self.visible_sessions().get(self.session_cursor).copied()
    }

    pub fn current_session_id(&self) -> Option<String> {
        self.current_session().map(|s| s.id.clone())
    }

    /// 목록이 줄어들었을 때 커서가 밖으로 나가지 않게 한다.
    pub fn clamp_cursors(&mut self) {
        let projects = self.result.projects.len();
        if projects == 0 {
            self.project_cursor = 0;
            self.session_cursor = 0;
            return;
        }
        self.project_cursor = self.project_cursor.min(projects - 1);
        let sessions = self.visible_sessions().len();
        self.session_cursor = self.session_cursor.min(sessions.saturating_sub(1));
    }

    pub fn session(&self, id: &str) -> Option<&crate::scan::session::Session> {
        self.result.sessions().find(|s| s.id == id)
    }

    pub fn recommended_ids(&self) -> Vec<String> {
        self.result
            .sessions()
            .filter(|s| self.verdicts.get(&s.id).is_some_and(|v| v.recommended()))
            .map(|s| s.id.clone())
            .collect()
    }

    /// 프로젝트별 추천 개수 — 왼쪽 패널에 표시한다.
    pub fn recommended_in(&self, project: &crate::scan::session::Project) -> usize {
        project
            .sessions
            .iter()
            .filter(|s| self.verdicts.get(&s.id).is_some_and(|v| v.recommended()))
            .count()
    }

    pub fn selected_in(&self, project: &crate::scan::session::Project) -> usize {
        project
            .sessions
            .iter()
            .filter(|s| self.selected.contains(&s.id))
            .count()
    }

    // ---------- 선택 ----------

    /// `Space`. 왼쪽 패널에서는 프로젝트의 세션을 한꺼번에, 오른쪽에서는 하나만.
    pub fn toggle_current(&mut self) {
        match self.focus {
            Focus::Sessions => {
                if let Some(id) = self.current_session_id() {
                    self.toggle_session(&id);
                }
            }
            Focus::Projects => {
                let ids: Vec<String> = self
                    .visible_sessions()
                    .iter()
                    .map(|s| s.id.clone())
                    .collect();
                if ids.is_empty() {
                    return;
                }
                let all_on = ids.iter().all(|i| self.selected.contains(i));
                for id in ids {
                    if all_on {
                        self.selected.remove(&id);
                    } else if self.can_select(&id) {
                        self.selected.insert(id);
                    }
                }
                self.status = format!("{}개 선택됨", self.selected.len());
            }
        }
    }

    pub fn toggle_session(&mut self, id: &str) {
        if self.selected.contains(id) {
            self.selected.remove(id);
        } else if self.can_select(id) {
            self.selected.insert(id.to_string());
        } else {
            let why = self
                .verdicts
                .get(id)
                .and_then(|v| v.blockers.first().map(|b| b.label()))
                .unwrap_or("선택할 수 없습니다");
            self.status = why.to_string();
        }
    }

    /// 차단된 세션은 사용자가 직접 골라도 선택되지 않는다.
    pub fn can_select(&self, id: &str) -> bool {
        self.verdicts.get(id).is_some_and(|v| v.cleanable())
    }

    /// `A`: 추천 항목 전체 선택·해제 (FR-05).
    pub fn toggle_all_recommended(&mut self) {
        let ids = self.recommended_ids();
        if ids.is_empty() {
            self.status = "추천 항목이 없습니다".into();
            return;
        }
        let all_on = ids.iter().all(|i| self.selected.contains(i));
        let count = ids.len();
        for id in ids {
            if all_on {
                self.selected.remove(&id);
            } else {
                self.selected.insert(id);
            }
        }
        // 지금 보고 있는 프로젝트뿐 아니라 전부가 대상이므로 범위를 밝힌다.
        let projects = self
            .result
            .projects
            .iter()
            .filter(|p| self.recommended_in(p) > 0)
            .count();
        self.status = if all_on {
            format!("추천 {count}개 선택 해제")
        } else {
            format!("추천 {count}개 선택 (프로젝트 {projects}개)")
        };
    }

    pub fn selected_targets(&self) -> Vec<CleanupTarget> {
        self.result
            .sessions()
            .filter(|s| self.selected.contains(&s.id))
            .map(|s| CleanupTarget {
                reasons: self
                    .verdicts
                    .get(&s.id)
                    .map(|v| v.reasons.iter().map(|r| r.label()).collect())
                    .unwrap_or_default(),
                session: s.clone(),
            })
            .collect()
    }

    // ---------- 이동 ----------

    pub fn move_cursor(&mut self, delta: isize) {
        match self.focus {
            Focus::Projects => {
                let last = self.visible_projects().len() as isize - 1;
                if last < 0 {
                    return;
                }
                let next = (self.project_cursor as isize + delta).clamp(0, last) as usize;
                if next != self.project_cursor {
                    self.project_cursor = next;
                    // 다른 프로젝트로 옮겼으니 세션 커서는 처음으로.
                    self.session_cursor = 0;
                }
            }
            Focus::Sessions => {
                let last = self.visible_sessions().len() as isize - 1;
                if last < 0 {
                    return;
                }
                self.session_cursor =
                    (self.session_cursor as isize + delta).clamp(0, last) as usize;
            }
        }
    }

    pub fn cursor_home(&mut self) {
        match self.focus {
            Focus::Projects => {
                self.project_cursor = 0;
                self.session_cursor = 0;
            }
            Focus::Sessions => self.session_cursor = 0,
        }
    }

    pub fn cursor_end(&mut self) {
        self.move_cursor(isize::MAX / 2);
    }

    /// `→` — 프로젝트에서 세션 목록으로.
    pub fn focus_sessions(&mut self) {
        if self.visible_sessions().is_empty() {
            self.status = "이 프로젝트에는 보여줄 세션이 없습니다".into();
            return;
        }
        self.focus = Focus::Sessions;
    }

    /// `←` — 세션에서 프로젝트 목록으로.
    pub fn focus_projects(&mut self) {
        self.focus = Focus::Projects;
    }

    // ---------- 정리 ----------

    pub fn open_confirm(&mut self) {
        let targets = self.selected_targets();
        if targets.is_empty() {
            self.status = "선택된 세션이 없습니다".into();
            return;
        }
        self.confirm = ConfirmState {
            preview: cleanup::preview(&targets, &self.live),
            mode: CleanupMode::Trash,
            typed: String::new(),
        };
        self.screen = Screen::Confirm;
    }

    pub fn set_mode(&mut self, mode: CleanupMode) {
        self.confirm.mode = mode;
        self.confirm.typed.clear();
    }

    pub fn run_cleanup(&mut self) {
        if !self.confirm.can_execute() {
            self.status = format!("계속하려면 {DELETE_WORD} 를 입력하세요");
            return;
        }
        let targets = self.selected_targets();
        let mode = self.confirm.mode;
        match cleanup::execute(&self.paths, targets, mode, &self.live) {
            Ok(outcome) => {
                self.status = format!(
                    "{} 완료 — 성공 {} · 제외 {} · 실패 {}",
                    mode.label(),
                    outcome.succeeded.len(),
                    outcome.skipped.len(),
                    outcome.failed.len()
                );
                self.outcome = Some(outcome);
                self.selected.clear();
            }
            Err(e) => {
                logging::error(&format!("cleanup aborted: {e:#}"));
                self.outcome = Some(CleanupOutcome {
                    failed: vec![("정리 작업".into(), format!("{e:#}"))],
                    ..Default::default()
                });
                self.status = format!("{e:#}");
            }
        }
        self.trash_ops = trash::list(&self.paths);
        self.rescan();
        self.screen = Screen::Result;
    }

    // ---------- 휴지통 ----------

    pub fn open_trash(&mut self) {
        self.trash_ops = trash::list(&self.paths);
        self.trash_cursor = 0;
        self.trash_selected.clear();
        self.screen = Screen::Trash;
    }

    /// 휴지통 화면의 행: (작업 인덱스, 세션 인덱스 또는 None = 작업 헤더)
    pub fn trash_rows(&self) -> Vec<(usize, Option<usize>)> {
        let mut rows = Vec::new();
        for (i, op) in self.trash_ops.iter().enumerate() {
            rows.push((i, None));
            for (j, _) in op.manifest.sessions.iter().enumerate() {
                rows.push((i, Some(j)));
            }
        }
        rows
    }

    pub fn toggle_trash_current(&mut self) {
        let rows = self.trash_rows();
        let Some(&(oi, si)) = rows.get(self.trash_cursor) else {
            return;
        };
        let op = &self.trash_ops[oi];
        let keys: Vec<(String, String)> = match si {
            Some(j) => vec![(
                op.manifest.op_id.clone(),
                op.manifest.sessions[j].session_id.clone(),
            )],
            None => op
                .manifest
                .sessions
                .iter()
                .map(|s| (op.manifest.op_id.clone(), s.session_id.clone()))
                .collect(),
        };
        let all_on = keys.iter().all(|k| self.trash_selected.contains(k));
        for k in keys {
            if all_on {
                self.trash_selected.remove(&k);
            } else {
                self.trash_selected.insert(k);
            }
        }
    }

    /// 선택이 없으면 커서가 놓인 작업 전체를 대상으로 삼는다.
    fn trash_selection(&self) -> HashMap<String, Vec<String>> {
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        if self.trash_selected.is_empty() {
            let rows = self.trash_rows();
            if let Some(&(oi, _)) = rows.get(self.trash_cursor) {
                let op = &self.trash_ops[oi];
                map.insert(
                    op.manifest.op_id.clone(),
                    op.manifest
                        .sessions
                        .iter()
                        .map(|s| s.session_id.clone())
                        .collect(),
                );
            }
            return map;
        }
        for (op_id, sid) in &self.trash_selected {
            map.entry(op_id.clone()).or_default().push(sid.clone());
        }
        map
    }

    pub fn restore_selection(&mut self) {
        let selection = self.trash_selection();
        if selection.is_empty() {
            self.status = "복원할 항목이 없습니다".into();
            return;
        }
        let mut combined = RestoreOutcome::default();
        for (op_id, ids) in selection {
            match trash::restore(&self.paths, &op_id, Some(&ids)) {
                Ok(o) => {
                    combined.restored.extend(o.restored);
                    combined.conflicts.extend(o.conflicts);
                    combined.failed.extend(o.failed);
                    combined.merged_shared += o.merged_shared;
                }
                Err(e) => combined.failed.push((op_id, format!("{e:#}"))),
            }
        }
        self.status = if combined.conflicts.is_empty() {
            format!("{}개 세션을 복원했습니다", combined.restored.len())
        } else {
            format!(
                "{}개 복원 · {}개는 같은 경로에 파일이 있어 건너뛰었습니다",
                combined.restored.len(),
                combined.conflicts.len()
            )
        };
        self.restore_outcome = Some(combined);
        self.trash_selected.clear();
        self.rescan();
        self.open_trash();
    }

    pub fn purge_selection(&mut self) {
        let selection = self.trash_selection();
        if selection.is_empty() {
            self.status = "삭제할 항목이 없습니다".into();
            return;
        }
        let mut freed = 0u64;
        for (op_id, ids) in selection {
            match trash::purge(&self.paths, &op_id, Some(&ids)) {
                Ok(b) => freed += b,
                Err(e) => self.status = format!("{e:#}"),
            }
        }
        self.status = format!(
            "휴지통에서 {} 를 영구 삭제했습니다",
            crate::ops::fsutil::human_bytes(freed)
        );
        self.trash_selected.clear();
        self.rescan();
        self.open_trash();
    }

    // ---------- 복구 ----------

    pub fn recover_pending(&mut self) {
        let ids: Vec<String> = self
            .pending_ops
            .iter()
            .map(|o| o.manifest.op_id.clone())
            .collect();
        let mut restored = 0;
        let mut conflicts = 0;
        for op_id in ids {
            match trash::recover(&self.paths, &op_id) {
                Ok(o) => {
                    restored += o.restored.len();
                    conflicts += o.conflicts.len();
                }
                Err(e) => self.status = format!("{e:#}"),
            }
        }
        self.pending_ops = trash::incomplete(&self.paths);
        self.rescan();
        self.status = format!("중단된 작업 복구: {restored}개 복원 · {conflicts}개 충돌");
        self.screen = Screen::Sessions;
    }

    pub fn skip_recovery(&mut self) {
        self.screen = Screen::Sessions;
        self.status = "복구를 건너뛰었습니다. 다음 실행에서 다시 안내합니다".into();
    }

    // ---------- 추천 기준 ----------

    pub const FILTER_ROWS: usize = 5;

    pub fn adjust_threshold(&mut self, delta: i64) {
        let next =
            (self.config.old_days as i64 + delta).clamp(MIN_OLD_DAYS as i64, MAX_OLD_DAYS as i64);
        self.config.old_days = next as u64;
        self.persist_config();
    }

    pub fn toggle_filter_row(&mut self) {
        match self.filter_cursor {
            0 => self.config.rule_old = !self.config.rule_old,
            1 => self.config.rule_short = !self.config.rule_short,
            2 => self.config.rule_subagent = !self.config.rule_subagent,
            3 => self.config.rule_missing_project = !self.config.rule_missing_project,
            _ => self.config.rule_orphan = !self.config.rule_orphan,
        }
        self.persist_config();
    }

    fn persist_config(&mut self) {
        if let Err(e) = self.config.save(&self.paths) {
            self.status = format!("설정을 저장하지 못했습니다: {e:#}");
        }
        self.recompute_verdicts();
        self.clamp_cursors();
    }

    pub fn total_selected_bytes(&self) -> u64 {
        self.result
            .sessions()
            .filter(|s| self.selected.contains(&s.id))
            .map(|s| s.size_bytes)
            .sum()
    }

    pub fn trash_total_sessions(&self) -> usize {
        self.trash_ops.iter().map(|o| o.session_count()).sum()
    }

    pub fn now(&self) -> i64 {
        self.now
    }

    pub fn is_orphan_project(key: &str) -> bool {
        key == ORPHAN_KEY
    }
}
