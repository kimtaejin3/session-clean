//! 테스트용 가짜 `~/.claude` 트리 빌더.
//!
//! PRD §16: "테스트는 실제 사용자 `~/.claude`가 아닌 임시 디렉터리의 Claude 데이터
//! fixture를 사용한다." 모든 통합 테스트는 이 빌더만 쓴다.

#![allow(dead_code)]

use sclean::paths::Paths;
use std::path::{Path, PathBuf};

pub const DAY: i64 = 86_400;

pub struct Fixture {
    pub dir: tempfile::TempDir,
}

impl Default for Fixture {
    fn default() -> Self {
        Self::new()
    }
}

impl Fixture {
    pub fn new() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let f = Fixture { dir };
        std::fs::create_dir_all(f.paths().projects_dir()).unwrap();
        f.paths().ensure_data_dirs().unwrap();
        f
    }

    /// Claude 데이터 디렉터리를 만들지 않은 상태 (PRD §14의 빈 상태 검증용).
    pub fn bare() -> Fixture {
        Fixture {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    pub fn paths(&self) -> Paths {
        Paths::with_roots(self.dir.path().join("claude"), self.dir.path().join("data"))
    }

    pub fn claude(&self) -> PathBuf {
        self.dir.path().join("claude")
    }

    /// 실제로 존재하는 프로젝트 소스 트리를 만든다 (FR-16 검증에 쓴다).
    pub fn source_tree(&self, name: &str) -> PathBuf {
        let p = self.dir.path().join("work").join(name);
        std::fs::create_dir_all(p.join("src")).unwrap();
        std::fs::write(p.join("src/main.rs"), b"fn main() {}").unwrap();
        p
    }

    pub fn session(&self, project_cwd: &str, id: &str) -> SessionBuilder {
        SessionBuilder {
            paths: self.paths(),
            project_key: encode_key(project_cwd),
            cwd: Some(project_cwd.to_string()),
            id: id.to_string(),
            lines: Vec::new(),
            age_secs: 0,
            subagent_of: None,
            extras: Vec::new(),
        }
    }

    /// `cwd` 없이 기록된 세션 — 프로젝트 존재 확인이 불가능한 경우 (PRD §11.1).
    pub fn session_without_cwd(&self, project_key: &str, id: &str) -> SessionBuilder {
        SessionBuilder {
            paths: self.paths(),
            project_key: project_key.to_string(),
            cwd: None,
            id: id.to_string(),
            lines: Vec::new(),
            age_secs: 0,
            subagent_of: None,
            extras: Vec::new(),
        }
    }

    pub fn write_history(&self, lines: &[&str]) {
        let p = self.paths().history_file();
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, format!("{}\n", lines.join("\n"))).unwrap();
    }

    pub fn read_history(&self) -> Vec<String> {
        std::fs::read_to_string(self.paths().history_file())
            .map(|t| t.lines().map(|s| s.to_string()).collect())
            .unwrap_or_default()
    }

    /// 살아있는 프로세스가 이 세션을 잡고 있는 것처럼 만든다.
    pub fn mark_running(&self, id: &str) {
        let dir = self.paths().sessions_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(format!("{}.json", std::process::id())),
            format!(r#"{{"sessionId":"{id}"}}"#),
        )
        .unwrap();
    }

    /// 대화 기록 없이 연결 데이터만 남긴다 (R5).
    pub fn orphan_env(&self, id: &str) {
        self.orphan_env_aged(id, 0);
    }

    pub fn orphan_env_aged(&self, id: &str, age_days: i64) {
        let d = self.paths().sidecar_dir("session-env").join(id);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("hook.sh"), b"echo hi").unwrap();
        if age_days > 0 {
            set_age(&d.join("hook.sh"), age_days * DAY);
            set_age(&d, age_days * DAY);
        }
    }

    pub fn orphan_task(&self, prefix: &str) {
        let d = self
            .paths()
            .sidecar_dir("tasks")
            .join(format!("session-{prefix}"));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("task.json"), b"{}").unwrap();
    }
}

pub struct SessionBuilder {
    paths: Paths,
    project_key: String,
    cwd: Option<String>,
    id: String,
    lines: Vec<String>,
    age_secs: i64,
    subagent_of: Option<String>,
    extras: Vec<&'static str>,
}

impl SessionBuilder {
    pub fn user(mut self, text: &str) -> Self {
        self.lines.push(self.line("user", &json_user(text)));
        self
    }

    pub fn tool_result(mut self, text: &str) -> Self {
        let body = format!(
            r#""message":{{"content":[{{"type":"tool_result","content":{}}}]}}"#,
            quote(text)
        );
        self.lines.push(self.line("user", &body));
        self
    }

    pub fn assistant(mut self, text: &str) -> Self {
        let body = format!(
            r#""message":{{"content":[{{"type":"text","text":{}}}]}}"#,
            quote(text)
        );
        self.lines.push(self.line("assistant", &body));
        self
    }

    pub fn tool_use(mut self, name: &str) -> Self {
        let body = format!(
            r#""message":{{"content":[{{"type":"tool_use","name":{},"input":{{}}}}]}}"#,
            quote(name)
        );
        self.lines.push(self.line("assistant", &body));
        self
    }

    pub fn summary(mut self, text: &str) -> Self {
        self.lines
            .push(format!(r#"{{"type":"summary","summary":{}}}"#, quote(text)));
        self
    }

    /// 깨진 줄을 그대로 주입한다 (PRD §14).
    pub fn raw_line(mut self, line: &str) -> Self {
        self.lines.push(line.to_string());
        self
    }

    pub fn sidechain(mut self) -> Self {
        self.subagent_of = Some("inline".to_string());
        self
    }

    /// `projects/<p>/<parent>/subagents/` 아래에 놓는다.
    pub fn subagent_of(mut self, parent_id: &str) -> Self {
        self.subagent_of = Some(parent_id.to_string());
        self
    }

    pub fn age_days(mut self, days: i64) -> Self {
        self.age_secs = days * DAY;
        self
    }

    pub fn age_secs(mut self, secs: i64) -> Self {
        self.age_secs = secs;
        self
    }

    pub fn with_task(mut self) -> Self {
        self.extras.push("tasks");
        self
    }

    pub fn with_team(mut self) -> Self {
        self.extras.push("teams");
        self
    }

    pub fn with_env(mut self) -> Self {
        self.extras.push("session-env");
        self
    }

    pub fn with_file_history(mut self) -> Self {
        self.extras.push("file-history");
        self
    }

    fn line(&self, kind: &str, body: &str) -> String {
        let sidechain = if self.subagent_of.is_some() {
            r#","isSidechain":true"#
        } else {
            ""
        };
        let cwd = match &self.cwd {
            Some(c) => format!(r#","cwd":{}"#, quote(c)),
            None => String::new(),
        };
        format!(
            r#"{{"type":{},"sessionId":{}{cwd}{sidechain},{body}}}"#,
            quote(kind),
            quote(&self.id)
        )
    }

    pub fn build(self) -> String {
        let proj = self.paths.projects_dir().join(&self.project_key);
        let dir = match &self.subagent_of {
            Some(parent) if parent != "inline" => {
                let d = proj.join(parent).join("subagents");
                std::fs::create_dir_all(&d).unwrap();
                d
            }
            _ => {
                std::fs::create_dir_all(&proj).unwrap();
                proj
            }
        };
        let file = dir.join(format!("{}.jsonl", self.id));
        let mut body = self.lines.join("\n");
        if !body.is_empty() {
            body.push('\n');
        }
        std::fs::write(&file, body).unwrap();

        for extra in &self.extras {
            let target = match *extra {
                "tasks" | "teams" => self
                    .paths
                    .sidecar_dir(extra)
                    .join(format!("session-{}", &self.id[..8])),
                other => self.paths.sidecar_dir(other).join(&self.id),
            };
            std::fs::create_dir_all(&target).unwrap();
            std::fs::write(target.join("data.json"), b"{\"a\":1}").unwrap();
            if self.age_secs > 0 {
                set_age(&target.join("data.json"), self.age_secs);
                set_age(&target, self.age_secs);
            }
        }

        if self.age_secs > 0 {
            set_age(&file, self.age_secs);
        }
        self.id
    }
}

/// `/Users/a/dev/b` -> `-Users-a-dev-b` (Claude의 인코딩 규칙).
pub fn encode_key(cwd: &str) -> String {
    cwd.replace('/', "-")
}

pub fn quote(s: &str) -> String {
    serde_json::to_string(s).unwrap()
}

fn json_user(text: &str) -> String {
    format!(r#""message":{{"role":"user","content":{}}}"#, quote(text))
}

/// mtime/atime을 과거로 돌린다 (R1 검증용).
pub fn set_age(path: &Path, age_secs: i64) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let t = now - age_secs;
    let times = [
        libc::timeval {
            tv_sec: t as libc::time_t,
            tv_usec: 0,
        },
        libc::timeval {
            tv_sec: t as libc::time_t,
            tv_usec: 0,
        },
    ];
    let c = std::ffi::CString::new(path.to_string_lossy().as_bytes()).unwrap();
    let rc = unsafe { libc::utimes(c.as_ptr(), times.as_ptr()) };
    assert_eq!(rc, 0, "utimes 실패: {}", path.display());
}

pub fn uuid(seed: u32) -> String {
    format!("{seed:08x}-1111-2222-3333-444444444444")
}
