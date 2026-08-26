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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    Project { key: String },
    Session { project_key: String, id: String },
}

impl Row {
    pub fn session_id(&self) -> Option<&str> {
        match self {
            Row::Session { id, .. } => Some(id),
            Row::Project { .. } => None,
        }
    }

    pub fn project_key(&self) -> &str {
        match self {
            Row::Project { key } => key,
            Row::Session { project_key, .. } => project_key,
        }
    }
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

    pub rows: Vec<Row>,
    pub cursor: usize,
    pub selected: HashSet<String>,
    pub collapsed: HashSet<String>,

    pub search: String,
    pub searching: bool,

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
            rows: Vec::new(),
            cursor: 0,
            selected: HashSet::new(),
            collapsed: HashSet::new(),
            search: String::new(),
            searching: false,
            screen,
            previous_screen: Screen::Sessions,
            focus: Focus::Sessions,
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
                self.rebuild_rows();
                // PRD §6: 시작 시 추천 세션을 자동으로 선택하지 않는다.
                self.selected.clear();
                let n = self.result.session_count();
                let rec = self.recommended_ids().len();
                self.status = format!("세션 {n}개 · 추천 {rec}개 (아무것도 선택되지 않음)");
                logging::info(&format!("scan complete sessions={n} recommended={rec}"));
            }
        }
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

    // ---------- 행 구성 ----------

    pub fn rebuild_rows(&mut self) {
        let needle = self.search.to_lowercase();
        let mut rows = Vec::new();
        for project in &self.result.projects {
            let project_hit = needle.is_empty()
                || project.label.to_lowercase().contains(&needle)
                || project.short_label().to_lowercase().contains(&needle);
            let matching: Vec<&crate::scan::session::Session> = project
                .sessions
                .iter()
                .filter(|s| project_hit || s.matches(&needle))
                .collect();
            if matching.is_empty() {
                continue;
            }
            rows.push(Row::Project {
                key: project.key.clone(),
            });
            if self.collapsed.contains(&project.key) {
                continue;
            }
            for s in matching {
                rows.push(Row::Session {
                    project_key: project.key.clone(),
                    id: s.id.clone(),
                });
            }
        }
        self.rows = rows;
        if self.cursor >= self.rows.len() {
            self.cursor = self.rows.len().saturating_sub(1);
        }
    }

    pub fn current_row(&self) -> Option<&Row> {
        self.rows.get(self.cursor)
    }

    pub fn session(&self, id: &str) -> Option<&crate::scan::session::Session> {
        self.result.sessions().find(|s| s.id == id)
    }

    /// 화면에 보이는 세션 중 추천 대상.
    pub fn visible_recommended(&self) -> Vec<String> {
        self.rows
            .iter()
            .filter_map(|r| r.session_id())
            .filter(|id| self.verdicts.get(*id).is_some_and(|v| v.recommended()))
            .map(|s| s.to_string())
            .collect()
    }

    pub fn recommended_ids(&self) -> Vec<String> {
        self.verdicts
            .iter()
            .filter(|(_, v)| v.recommended())
            .map(|(k, _)| k.clone())
            .collect()
    }

    // ---------- 선택 ----------

    pub fn toggle_current(&mut self) {
        let Some(row) = self.rows.get(self.cursor).cloned() else {
            return;
        };
        match row {
            Row::Session { id, .. } => self.toggle_session(&id),
            Row::Project { key } => {
                // 프로젝트 행에서는 그 프로젝트의 정리 가능한 세션을 한꺼번에 토글한다.
                let ids: Vec<String> = self
                    .rows
                    .iter()
                    .filter_map(|r| match r {
                        Row::Session { project_key, id } if *project_key == key => Some(id.clone()),
                        _ => None,
                    })
                    .collect();
                let all_on = ids.iter().all(|i| self.selected.contains(i));
                for id in ids {
                    if all_on {
                        self.selected.remove(&id);
                    } else if self.can_select(&id) {
                        self.selected.insert(id);
                    }
                }
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

    /// `A`: 현재 필터에서 추천 항목 전체 선택·해제 (FR-05).
    pub fn toggle_all_recommended(&mut self) {
        let ids = self.visible_recommended();
        if ids.is_empty() {
            self.status = "현재 화면에 추천 항목이 없습니다".into();
            return;
        }
        let all_on = ids.iter().all(|i| self.selected.contains(i));
        for id in ids {
            if all_on {
                self.selected.remove(&id);
            } else {
                self.selected.insert(id);
            }
        }
        self.status = format!("{}개 선택됨", self.selected.len());
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
        if self.rows.is_empty() {
            return;
        }
        let last = self.rows.len() as isize - 1;
        let next = (self.cursor as isize + delta).clamp(0, last);
        self.cursor = next as usize;
    }

    pub fn collapse_current(&mut self) {
        if let Some(row) = self.rows.get(self.cursor).cloned() {
            let key = row.project_key().to_string();
            self.collapsed.insert(key);
            self.rebuild_rows();
        }
    }

    pub fn expand_current(&mut self) {
        if let Some(row) = self.rows.get(self.cursor).cloned() {
            self.collapsed.remove(row.project_key());
            self.rebuild_rows();
        }
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
        self.rebuild_rows();
    }

    // ---------- 검색 ----------

    pub fn start_search(&mut self) {
        self.searching = true;
    }

    pub fn push_search(&mut self, c: char) {
        self.search.push(c);
        self.rebuild_rows();
    }

    pub fn pop_search(&mut self) {
        self.search.pop();
        self.rebuild_rows();
    }

    pub fn clear_search(&mut self) {
        self.search.clear();
        self.searching = false;
        self.rebuild_rows();
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
