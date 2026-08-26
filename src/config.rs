//! 추천 기준 설정. PRD §8.4 — TUI에서 바꾸고 `config.json`에 저장한다.

use crate::paths::Paths;
use serde::{Deserialize, Serialize};

pub const DEFAULT_OLD_DAYS: u64 = 30;
pub const MIN_OLD_DAYS: u64 = 1;
pub const MAX_OLD_DAYS: u64 = 3650;

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(default)]
pub struct Config {
    /// R1 오래됨 기준 일수.
    pub old_days: u64,
    pub rule_old: bool,
    pub rule_missing_project: bool,
    pub rule_short: bool,
    pub rule_subagent: bool,
    pub rule_orphan: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            old_days: DEFAULT_OLD_DAYS,
            rule_old: true,
            rule_missing_project: true,
            rule_short: true,
            rule_subagent: true,
            rule_orphan: true,
        }
    }
}

impl Config {
    /// 파일이 없거나 손상됐으면 조용히 기본값으로 되돌린다.
    /// 설정 파일 하나 때문에 도구가 못 뜨면 안 된다.
    pub fn load(paths: &Paths) -> Config {
        let Ok(text) = std::fs::read_to_string(paths.config_file()) else {
            return Config::default();
        };
        match serde_json::from_str::<Config>(&text) {
            Ok(mut cfg) => {
                cfg.old_days = cfg.old_days.clamp(MIN_OLD_DAYS, MAX_OLD_DAYS);
                cfg
            }
            Err(_) => Config::default(),
        }
    }

    pub fn save(&self, paths: &Paths) -> anyhow::Result<()> {
        paths.ensure_data_dirs()?;
        let text = serde_json::to_vec_pretty(self)?;
        crate::ops::fsutil::atomic_write(&paths.config_file(), &text)?;
        Ok(())
    }

    /// 활성화된 규칙이 하나도 없으면 추천 자체가 없다는 뜻이다(유효한 상태).
    pub fn any_rule_enabled(&self) -> bool {
        self.rule_old
            || self.rule_missing_project
            || self.rule_short
            || self.rule_subagent
            || self.rule_orphan
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn paths(tmp: &tempfile::TempDir) -> Paths {
        Paths::with_roots(tmp.path().join("c"), tmp.path().join("d"))
    }

    #[test]
    fn defaults_match_the_prd() {
        let cfg = Config::default();
        assert_eq!(cfg.old_days, 30);
        assert!(cfg.rule_old && cfg.rule_missing_project);
        assert!(cfg.rule_short && cfg.rule_subagent && cfg.rule_orphan);
    }

    #[test]
    fn round_trips_and_recovers_from_corruption() {
        let tmp = tempfile::tempdir().unwrap();
        let p = paths(&tmp);
        let cfg = Config {
            old_days: 92,
            rule_short: false,
            ..Config::default()
        };
        cfg.save(&p).unwrap();
        assert_eq!(Config::load(&p), cfg);

        std::fs::write(p.config_file(), b"{ not json").unwrap();
        assert_eq!(Config::load(&p), Config::default());
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let p = paths(&tmp);
        p.ensure_data_dirs().unwrap();
        std::fs::write(p.config_file(), br#"{"old_days": 7, "unknown": 1}"#).unwrap();
        let cfg = Config::load(&p);
        assert_eq!(cfg.old_days, 7);
        assert!(cfg.rule_short, "명시되지 않은 규칙은 기본값 유지");
    }

    #[test]
    fn absurd_thresholds_are_clamped() {
        let tmp = tempfile::tempdir().unwrap();
        let p = paths(&tmp);
        p.ensure_data_dirs().unwrap();
        std::fs::write(p.config_file(), br#"{"old_days": 0}"#).unwrap();
        assert_eq!(Config::load(&p).old_days, MIN_OLD_DAYS);
    }

    #[test]
    fn config_file_lives_in_application_support() {
        let p = Paths::with_roots(PathBuf::from("/c"), PathBuf::from("/d"));
        assert!(p.config_file().ends_with("config.json"));
    }
}
