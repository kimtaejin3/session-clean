//! Claude 데이터와 sclean 저장소의 위치를 결정하는 단일 지점.
//!
//! PRD §15 호환성: "Claude 데이터 위치와 필드 처리는 한곳에서 교체할 수 있어야 한다."
//! 경로에 관한 모든 지식은 이 모듈에만 존재한다. 테스트는 `with_roots`로
//! 임시 디렉터리를 주입해 실제 `~/.claude`를 건드리지 않는다.

use std::path::{Path, PathBuf};

/// Claude 데이터 디렉터리 중 세션에 연결될 수 있는 하위 디렉터리 이름.
///
/// PRD §11.1의 읽기 대상 목록. 실제 설치에 없는 경로는 오류가 아니다.
pub const SIDECAR_DIRS: &[&str] = &[
    "tasks",
    "todos",
    "sessions",
    "session-env",
    "teams",
    "file-history",
    "debug",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    /// Claude Code 로컬 데이터 루트 (`~/.claude`).
    pub claude_dir: PathBuf,
    /// sclean 자체 저장소 (`~/Library/Application Support/sclean`).
    pub data_dir: PathBuf,
}

impl Paths {
    /// 환경변수 재정의를 우선하고, 없으면 홈 디렉터리 기준 기본 위치를 쓴다.
    ///
    /// `SCLEAN_CLAUDE_DIR` / `SCLEAN_DATA_DIR`는 수동 검증과 fixture 실행에 쓴다.
    pub fn discover() -> anyhow::Result<Paths> {
        let home =
            dirs::home_dir().ok_or_else(|| anyhow::anyhow!("홈 디렉터리를 찾지 못했습니다"))?;
        let claude_dir = match std::env::var_os("SCLEAN_CLAUDE_DIR") {
            Some(v) => PathBuf::from(v),
            None => home.join(".claude"),
        };
        let data_dir = match std::env::var_os("SCLEAN_DATA_DIR") {
            Some(v) => PathBuf::from(v),
            None => default_data_dir(&home),
        };
        Ok(Paths {
            claude_dir,
            data_dir,
        })
    }

    pub fn with_roots(claude_dir: PathBuf, data_dir: PathBuf) -> Paths {
        Paths {
            claude_dir,
            data_dir,
        }
    }

    pub fn projects_dir(&self) -> PathBuf {
        self.claude_dir.join("projects")
    }

    pub fn history_file(&self) -> PathBuf {
        self.claude_dir.join("history.jsonl")
    }

    pub fn sessions_dir(&self) -> PathBuf {
        self.claude_dir.join("sessions")
    }

    pub fn sidecar_dir(&self, name: &str) -> PathBuf {
        self.claude_dir.join(name)
    }

    pub fn config_file(&self) -> PathBuf {
        self.data_dir.join("config.json")
    }

    pub fn log_file(&self) -> PathBuf {
        self.data_dir.join("sclean.log")
    }

    pub fn trash_dir(&self) -> PathBuf {
        self.data_dir.join("trash")
    }

    /// sclean 저장소를 준비한다. Claude 디렉터리는 절대 만들지 않는다.
    pub fn ensure_data_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(self.trash_dir())
    }

    /// Claude 데이터 루트가 실제로 존재하는지. 없으면 빈 상태를 보여준다(PRD §14).
    pub fn claude_dir_exists(&self) -> bool {
        self.claude_dir.is_dir()
    }
}

/// 플랫폼 관례에 맞는 sclean 저장 위치.
///
/// macOS는 PRD §11.2가 지정한 `~/Library/Application Support/sclean`,
/// Linux는 XDG 관례인 `~/.local/share/sclean`을 쓴다. `dirs::data_dir()`이
/// 두 경우를 모두 알고 있으므로 그것을 따르고, 알아내지 못하면 홈 기준으로 되돌린다.
fn default_data_dir(home: &Path) -> PathBuf {
    match dirs::data_dir() {
        Some(d) => d.join("sclean"),
        None => home.join(".local/share/sclean"),
    }
}

/// `projects/` 아래 인코딩된 디렉터리 이름을 사람이 읽을 수 있는 형태로 되돌린다.
///
/// Claude는 `/Users/a/dev/b`를 `-Users-a-dev-b`로 인코딩한다. 이 변환은 손실이
/// 있으므로(원래 경로에 `-`가 있으면 복원 불가) 오직 **표시용**이며, 프로젝트
/// 존재 여부 판정에는 절대 쓰지 않는다. 존재 판정은 세션 기록의 `cwd`만 쓴다.
pub fn decode_project_label(key: &str) -> String {
    if key.starts_with('-') {
        key.replacen('-', "/", 1).replace('-', "/")
    } else {
        key.to_string()
    }
}

/// 표시용 짧은 라벨: 마지막 경로 성분.
pub fn short_label(label: &str) -> String {
    Path::new(label)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| label.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_roots_derives_all_locations() {
        let p = Paths::with_roots(PathBuf::from("/c"), PathBuf::from("/d"));
        assert_eq!(p.projects_dir(), PathBuf::from("/c/projects"));
        assert_eq!(p.history_file(), PathBuf::from("/c/history.jsonl"));
        assert_eq!(p.sessions_dir(), PathBuf::from("/c/sessions"));
        assert_eq!(p.sidecar_dir("tasks"), PathBuf::from("/c/tasks"));
        assert_eq!(p.config_file(), PathBuf::from("/d/config.json"));
        assert_eq!(p.log_file(), PathBuf::from("/d/sclean.log"));
        assert_eq!(p.trash_dir(), PathBuf::from("/d/trash"));
    }

    #[test]
    fn default_data_dir_follows_platform_convention() {
        let home = PathBuf::from("/home/x");
        let dir = default_data_dir(&home);
        assert!(dir.ends_with("sclean"));
        if cfg!(target_os = "macos") {
            assert!(
                dir.to_string_lossy().contains("Application Support"),
                "macOS는 PRD가 지정한 위치를 쓴다: {}",
                dir.display()
            );
        } else {
            assert!(
                dir.to_string_lossy().contains(".local/share")
                    || dir.to_string_lossy().contains("share"),
                "Linux는 XDG 관례를 따른다: {}",
                dir.display()
            );
        }
    }

    #[test]
    fn env_override_wins_over_platform_default() {
        // SAFETY: 이 테스트는 자기 프로세스의 환경변수만 건드린다.
        unsafe {
            std::env::set_var("SCLEAN_DATA_DIR", "/tmp/sclean-test-override");
        }
        let p = Paths::discover().unwrap();
        assert_eq!(p.data_dir, PathBuf::from("/tmp/sclean-test-override"));
        unsafe {
            std::env::remove_var("SCLEAN_DATA_DIR");
        }
    }

    #[test]
    fn ensure_data_dirs_creates_trash_but_not_claude_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let p = Paths::with_roots(tmp.path().join("c"), tmp.path().join("d"));
        p.ensure_data_dirs().unwrap();
        assert!(p.trash_dir().is_dir());
        assert!(!p.claude_dir.exists());
    }

    #[test]
    fn decodes_project_label_for_display_only() {
        assert_eq!(
            decode_project_label("-Users-kim-dev-ev"),
            "/Users/kim/dev/ev"
        );
        assert_eq!(short_label("/Users/kim/dev/ev"), "ev");
    }
}
