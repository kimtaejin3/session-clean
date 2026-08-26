//! 규칙 기반 추천 (PRD §9).
//!
//! 핵심 원칙: **설명 가능해야 한다.** 각 추천에는 사용자가 눈으로 확인할 수 있는
//! 이유가 붙는다. 점수나 불투명한 판단은 쓰지 않는다.
//!
//! 세 가지 상태를 구분한다.
//! - `Reason`  — 추천하는 이유 (하나라도 있으면 후보)
//! - `Blocker` — 정리 자체를 **차단**한다. 사용자가 직접 골라도 실행하지 않는다.
//! - `Caution` — 추천만 하지 않는다. 사용자가 직접 고르면 정리할 수 있다.
//!
//! PRD §9는 "불확실한 세션은 추천하지 않는다"와 "정리를 차단한다"를 함께 적어 두었다.
//! 되돌릴 수 없는 위험(실행 중, 소유 불명, 형식 불명)은 차단하고, 단순히 정보가
//! 부족한 경우(프로젝트 경로 미확인, 방금 활동)는 추천만 보류한다.

use crate::config::Config;
use crate::live::LiveSessions;
use crate::scan::artifacts::ArtifactKind;
use crate::scan::session::{Session, SessionKind};
use std::path::PathBuf;

/// 이 시간 안에 활동한 세션은 아직 쓰이는 중일 수 있다.
pub const RECENT_ACTIVITY_SECS: i64 = 300;

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Reason {
    /// R1
    Old { days: u64 },
    /// R2
    MissingProject { path: PathBuf },
    /// R3
    ShortSession { user_messages: usize },
    /// R4
    FinishedSubagent,
    /// R5
    OrphanData { kinds: Vec<ArtifactKind> },
}

impl Reason {
    pub fn label(&self) -> String {
        match self {
            Reason::Old { days } => format!("마지막 활동 후 {days}일 경과"),
            Reason::MissingProject { path } => {
                format!("프로젝트 경로 없음: {}", path.display())
            }
            Reason::ShortSession { user_messages } => {
                format!("사용자 메시지 {user_messages}개, 도구 실행 없음")
            }
            Reason::FinishedSubagent => "종료된 하위 에이전트 세션".to_string(),
            Reason::OrphanData { kinds } => {
                let names: Vec<&str> = kinds.iter().map(|k| k.label()).collect();
                format!("대화 기록 없이 남은 데이터: {}", names.join(", "))
            }
        }
    }

    pub fn rule_id(&self) -> &'static str {
        match self {
            Reason::Old { .. } => "R1",
            Reason::MissingProject { .. } => "R2",
            Reason::ShortSession { .. } => "R3",
            Reason::FinishedSubagent => "R4",
            Reason::OrphanData { .. } => "R5",
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Blocker {
    /// 세션 형식을 분석할 수 없음
    Unparsable,
    /// 연결 데이터의 소유 세션을 확정할 수 없음
    AmbiguousOwnership,
    /// 현재 실행 중인 것으로 확인됨
    Running,
    /// 스캔 이후 파일이 바뀌었음 (실행 직전 재검증에서만 붙는다)
    ChangedSinceScan,
    /// 지울 파일이 하나도 없음
    NothingToClean,
}

impl Blocker {
    pub fn label(&self) -> &'static str {
        match self {
            Blocker::Unparsable => "세션 형식을 분석할 수 없어 정리하지 않습니다",
            Blocker::AmbiguousOwnership => "연결 데이터의 소유 세션을 확정할 수 없습니다",
            Blocker::Running => "지금 실행 중인 세션입니다",
            Blocker::ChangedSinceScan => "스캔 이후 파일이 변경되었습니다",
            Blocker::NothingToClean => "정리할 파일이 없습니다",
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Caution {
    /// 프로젝트 경로를 신뢰할 수 있게 확인하지 못함
    ProjectUnverified,
    /// 방금 전까지 활동함
    RecentlyActive,
}

impl Caution {
    pub fn label(&self) -> &'static str {
        match self {
            Caution::ProjectUnverified => "프로젝트 경로 확인 불가",
            Caution::RecentlyActive => "최근 활동 중",
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Verdict {
    pub reasons: Vec<Reason>,
    pub blockers: Vec<Blocker>,
    pub cautions: Vec<Caution>,
}

impl Verdict {
    /// 자동 추천 대상인가. PRD §6: "불확실한 세션은 추천하지 않는다."
    pub fn recommended(&self) -> bool {
        !self.reasons.is_empty() && self.blockers.is_empty() && self.cautions.is_empty()
    }

    /// 사용자가 직접 골랐을 때 정리할 수 있는가.
    pub fn cleanable(&self) -> bool {
        self.blockers.is_empty()
    }

    /// 화면에 보여줄 이유 문구.
    pub fn label(&self) -> String {
        if let Some(b) = self.blockers.first() {
            return b.label().to_string();
        }
        if self.reasons.is_empty() {
            return match self.cautions.first() {
                Some(c) => c.label().to_string(),
                None => String::new(),
            };
        }
        let mut text = self
            .reasons
            .iter()
            .map(|r| r.label())
            .collect::<Vec<_>>()
            .join(" · ");
        if let Some(c) = self.cautions.first() {
            text.push_str(&format!(" (보류: {})", c.label()));
        }
        text
    }

    /// 로그용 — 프롬프트 본문이 절대 섞이지 않는 요약 (PRD §15).
    pub fn rule_ids(&self) -> String {
        self.reasons
            .iter()
            .map(|r| r.rule_id())
            .collect::<Vec<_>>()
            .join(",")
    }
}

pub fn evaluate(s: &Session, cfg: &Config, now_secs: i64, live: &LiveSessions) -> Verdict {
    let mut v = Verdict::default();

    // --- 차단 조건 먼저 (PRD §9 추천 제외 조건) ---
    if s.analysis.is_unreadable() {
        v.blockers.push(Blocker::Unparsable);
    }
    if s.ambiguous_ownership {
        v.blockers.push(Blocker::AmbiguousOwnership);
    }
    if live.contains(&s.id) {
        v.blockers.push(Blocker::Running);
    }
    if s.artifacts.is_empty() {
        v.blockers.push(Blocker::NothingToClean);
    }

    // --- 주의 조건 ---
    let idle = now_secs.saturating_sub(s.last_active_secs);
    if idle < RECENT_ACTIVITY_SECS {
        v.cautions.push(Caution::RecentlyActive);
    }
    if s.project_exists.is_none() && s.kind != SessionKind::Orphan {
        v.cautions.push(Caution::ProjectUnverified);
    }

    // --- 규칙 ---
    // R1 오래된 세션
    if cfg.rule_old {
        let days = idle.max(0) / 86_400;
        if days as u64 > cfg.old_days {
            v.reasons.push(Reason::Old { days: days as u64 });
        }
    }

    // R2 존재하지 않는 프로젝트 — 경로를 확인하지 못했으면 적용하지 않는다.
    if cfg.rule_missing_project
        && s.project_exists == Some(false)
        && let Some(path) = &s.project_path
    {
        v.reasons.push(Reason::MissingProject { path: path.clone() });
    }

    // R3 짧은 세션 — JSONL을 정상적으로 분석한 경우에만.
    if cfg.rule_short
        && s.kind != SessionKind::Orphan
        && s.analysis.is_usable()
        && let Some(info) = s.analysis.info()
        && info.user_messages <= 1
        && info.tool_uses == 0
    {
        v.reasons.push(Reason::ShortSession {
            user_messages: info.user_messages,
        });
    }

    // R4 종료된 하위 에이전트 — 최근 변경 중인 것은 제외한다.
    if cfg.rule_subagent
        && s.kind == SessionKind::Subagent
        && idle >= RECENT_ACTIVITY_SECS
        && !s.analysis.is_unreadable()
    {
        v.reasons.push(Reason::FinishedSubagent);
    }

    // R5 고아 데이터 — 세션 ID로 정확히 연결될 때만.
    if cfg.rule_orphan && s.kind == SessionKind::Orphan && !s.ambiguous_ownership {
        v.reasons.push(Reason::OrphanData {
            kinds: s.artifact_kinds(),
        });
    }

    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::jsonl::{Analysis, ParsedInfo};

    fn session(kind: SessionKind) -> Session {
        Session {
            id: "aaaaaaaa-1111-2222-3333-444444444444".into(),
            project_key: "-w".into(),
            transcript: None,
            project_path: Some(PathBuf::from("/w")),
            project_exists: Some(true),
            display_name: "테스트".into(),
            last_active_secs: 0,
            size_bytes: 10,
            analysis: Analysis::Parsed(ParsedInfo {
                user_messages: 5,
                tool_uses: 3,
                ..Default::default()
            }),
            kind,
            artifacts: vec![dummy_artifact()],
            ambiguous_ownership: false,
        }
    }

    fn dummy_artifact() -> crate::scan::artifacts::Artifact {
        crate::scan::artifacts::Artifact {
            path: PathBuf::from("/w/x"),
            kind: ArtifactKind::Transcript,
            is_dir: false,
            size: 10,
            fingerprint: Default::default(),
        }
    }

    const NOW: i64 = 100 * 86_400;

    fn days_ago(d: i64) -> i64 {
        NOW - d * 86_400
    }

    #[test]
    fn r1_matches_only_beyond_threshold() {
        let cfg = Config::default(); // 30일
        let mut s = session(SessionKind::Normal);

        s.last_active_secs = days_ago(31);
        let v = evaluate(&s, &cfg, NOW, &LiveSessions::empty());
        assert_eq!(v.reasons, vec![Reason::Old { days: 31 }]);
        assert!(v.recommended());

        s.last_active_secs = days_ago(30);
        assert!(
            evaluate(&s, &cfg, NOW, &LiveSessions::empty())
                .reasons
                .is_empty(),
            "기준과 같으면 아직 오래된 것이 아니다"
        );
    }

    #[test]
    fn reason_labels_are_explainable_korean() {
        assert_eq!(Reason::Old { days: 92 }.label(), "마지막 활동 후 92일 경과");
    }

    #[test]
    fn r2_skipped_when_project_path_unverified() {
        let cfg = Config::default();
        let mut s = session(SessionKind::Normal);
        s.last_active_secs = days_ago(1);
        s.project_exists = None;
        let v = evaluate(&s, &cfg, NOW, &LiveSessions::empty());
        assert!(v.reasons.is_empty(), "확인 불가는 '없음'이 아니다");
        assert!(v.cautions.contains(&Caution::ProjectUnverified));

        s.project_exists = Some(false);
        let v = evaluate(&s, &cfg, NOW, &LiveSessions::empty());
        assert_eq!(
            v.reasons,
            vec![Reason::MissingProject {
                path: PathBuf::from("/w")
            }]
        );
        assert!(v.recommended());
    }

    #[test]
    fn r3_requires_parsable_analysis() {
        let cfg = Config::default();
        let mut s = session(SessionKind::Normal);
        s.last_active_secs = days_ago(1);
        s.analysis = Analysis::Parsed(ParsedInfo {
            user_messages: 1,
            tool_uses: 0,
            ..Default::default()
        });
        let v = evaluate(&s, &cfg, NOW, &LiveSessions::empty());
        assert_eq!(v.reasons, vec![Reason::ShortSession { user_messages: 1 }]);

        // 같은 내용이지만 일부 줄이 깨진 경우 — 추천하지 않는다.
        s.analysis = Analysis::Partial(ParsedInfo {
            user_messages: 1,
            tool_uses: 0,
            broken_lines: 1,
            ..Default::default()
        });
        assert!(
            evaluate(&s, &cfg, NOW, &LiveSessions::empty())
                .reasons
                .is_empty()
        );
    }

    #[test]
    fn r4_excludes_recently_active_subagents() {
        let cfg = Config::default();
        let mut s = session(SessionKind::Subagent);
        s.last_active_secs = NOW - 10; // 방금 전
        let v = evaluate(&s, &cfg, NOW, &LiveSessions::empty());
        assert!(!v.reasons.contains(&Reason::FinishedSubagent));
        assert!(v.cautions.contains(&Caution::RecentlyActive));

        s.last_active_secs = days_ago(2);
        let v = evaluate(&s, &cfg, NOW, &LiveSessions::empty());
        assert!(v.reasons.contains(&Reason::FinishedSubagent));
    }

    #[test]
    fn r5_requires_unambiguous_ownership() {
        let cfg = Config::default();
        let mut s = session(SessionKind::Orphan);
        s.last_active_secs = days_ago(1);
        let v = evaluate(&s, &cfg, NOW, &LiveSessions::empty());
        assert!(matches!(v.reasons[0], Reason::OrphanData { .. }));
        assert!(v.recommended());

        s.ambiguous_ownership = true;
        let v = evaluate(&s, &cfg, NOW, &LiveSessions::empty());
        assert!(!v.recommended());
        assert!(!v.cleanable());
        assert!(v.blockers.contains(&Blocker::AmbiguousOwnership));
    }

    #[test]
    fn unparsable_session_is_never_recommended_and_not_cleanable() {
        let cfg = Config::default();
        let mut s = session(SessionKind::Normal);
        s.last_active_secs = days_ago(999);
        s.analysis = Analysis::Unreadable("형식 불명".into());
        let v = evaluate(&s, &cfg, NOW, &LiveSessions::empty());
        assert!(!v.recommended());
        assert!(!v.cleanable());
        assert_eq!(v.label(), Blocker::Unparsable.label());
    }

    #[test]
    fn running_session_is_blocked_even_when_old() {
        let cfg = Config::default();
        let mut s = session(SessionKind::Normal);
        s.last_active_secs = days_ago(400);
        // 감지된 실행 중 세션을 흉내낸다.
        let tmp = tempfile::tempdir().unwrap();
        let paths = crate::Paths::with_roots(tmp.path().join("c"), tmp.path().join("d"));
        std::fs::create_dir_all(paths.sessions_dir()).unwrap();
        std::fs::write(
            paths.sessions_dir().join(format!("{}.json", std::process::id())),
            format!(r#"{{"sessionId":"{}"}}"#, s.id),
        )
        .unwrap();
        let live = LiveSessions::detect(&paths);

        let v = evaluate(&s, &cfg, NOW, &live);
        assert!(v.blockers.contains(&Blocker::Running));
        assert!(!v.cleanable());
    }

    #[test]
    fn disabled_rules_produce_no_reasons() {
        let cfg = Config {
            old_days: 30,
            rule_old: false,
            rule_missing_project: false,
            rule_short: false,
            rule_subagent: false,
            rule_orphan: false,
        };
        let mut s = session(SessionKind::Subagent);
        s.last_active_secs = days_ago(999);
        s.project_exists = Some(false);
        s.analysis = Analysis::Parsed(ParsedInfo::default());
        assert!(
            evaluate(&s, &cfg, NOW, &LiveSessions::empty())
                .reasons
                .is_empty()
        );
    }

    #[test]
    fn multiple_rules_stack_into_one_explanation() {
        let cfg = Config::default();
        let mut s = session(SessionKind::Subagent);
        s.last_active_secs = days_ago(92);
        s.project_exists = Some(false);
        let v = evaluate(&s, &cfg, NOW, &LiveSessions::empty());
        assert_eq!(v.reasons.len(), 3, "{:?}", v.reasons);
        assert!(v.label().contains("마지막 활동 후 92일 경과"));
        assert!(v.label().contains("프로젝트 경로 없음"));
        assert_eq!(v.rule_ids(), "R1,R2,R4");
    }

    #[test]
    fn a_session_with_no_files_is_blocked() {
        let cfg = Config::default();
        let mut s = session(SessionKind::Normal);
        s.artifacts.clear();
        s.last_active_secs = days_ago(999);
        let v = evaluate(&s, &cfg, NOW, &LiveSessions::empty());
        assert!(v.blockers.contains(&Blocker::NothingToClean));
    }
}
