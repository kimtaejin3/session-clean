//! 세션 ID에서 연결 데이터를 찾아내고 소유권을 판정한다.
//!
//! 실측한 Claude 레이아웃(2026-08):
//! ```text
//! projects/<encoded-cwd>/<uuid>.jsonl        전체 UUID — 단독 소유
//! projects/<encoded-cwd>/<uuid>/             전체 UUID — 단독 소유 (subagents/ 포함)
//! session-env/<uuid>/                        전체 UUID — 단독 소유
//! file-history/<uuid>/                       전체 UUID — 단독 소유
//! todos/<uuid>*                              전체 UUID — 단독 소유
//! tasks/session-<uuid[0..8]>/                8자 접두어 — 유일할 때만 소유
//! teams/session-<uuid[0..8]>/                8자 접두어 — 유일할 때만 소유
//! debug/*<uuid>*                             전체 UUID 포함 — 단독 소유
//! ```
//!
//! PRD §9 추천 제외 조건: "연결 데이터의 소유 세션을 확정할 수 없음."
//! 접두어가 두 세션 이상에 걸리면 그 세션의 정리를 아예 차단한다.

use crate::ops::fsutil;
use crate::paths::Paths;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub const PREFIX_LEN: usize = 8;
/// `tasks/`, `teams/`에서 세션 키에 붙는 접두사.
pub const SESSION_PREFIX: &str = "session-";

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ArtifactKind {
    Transcript,
    SessionDir,
    Task,
    Team,
    SessionEnv,
    FileHistory,
    Todo,
    Debug,
}

impl ArtifactKind {
    pub fn label(&self) -> &'static str {
        match self {
            ArtifactKind::Transcript => "대화 기록",
            ArtifactKind::SessionDir => "세션 폴더",
            ArtifactKind::Task => "작업",
            ArtifactKind::Team => "팀",
            ArtifactKind::SessionEnv => "환경",
            ArtifactKind::FileHistory => "파일 이력",
            ArtifactKind::Todo => "할 일",
            ArtifactKind::Debug => "디버그",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Fingerprint {
    pub size: u64,
    pub mtime_secs: i64,
    pub mtime_nanos: u32,
}

impl Fingerprint {
    /// 링크를 따라가지 않고 항목 자체의 지문을 만든다.
    /// 디렉터리는 크기 대신 재귀 합계를 쓴다 — 안의 파일이 바뀌면 감지된다.
    pub fn of(path: &Path) -> Option<Fingerprint> {
        let meta = std::fs::symlink_metadata(path).ok()?;
        let mtime = meta.modified().ok()?;
        let dur = mtime
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let size = if meta.is_dir() {
            fsutil::entry_size(path)
        } else {
            meta.len()
        };
        Some(Fingerprint {
            size,
            mtime_secs: dur.as_secs() as i64,
            mtime_nanos: dur.subsec_nanos(),
        })
    }
}

#[derive(Clone, Debug)]
pub struct Artifact {
    pub path: PathBuf,
    pub kind: ArtifactKind,
    pub is_dir: bool,
    pub size: u64,
    pub fingerprint: Fingerprint,
}

impl Artifact {
    fn at(path: PathBuf, kind: ArtifactKind) -> Option<Artifact> {
        let meta = std::fs::symlink_metadata(&path).ok()?;
        let fingerprint = Fingerprint::of(&path)?;
        Some(Artifact {
            is_dir: meta.is_dir(),
            size: fingerprint.size,
            path,
            kind,
            fingerprint,
        })
    }

    /// 스캔 이후 바뀌지 않았는가(FR-13).
    pub fn unchanged(&self) -> bool {
        Fingerprint::of(&self.path) == Some(self.fingerprint)
    }

    pub fn still_exists(&self) -> bool {
        std::fs::symlink_metadata(&self.path).is_ok()
    }
}

/// UUID 앞 8자 -> 그 접두어를 가진 세션 ID 목록.
#[derive(Debug, Default)]
pub struct PrefixIndex(HashMap<String, Vec<String>>);

impl PrefixIndex {
    pub fn build<I, S>(session_ids: I) -> PrefixIndex
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        for id in session_ids {
            let id = id.as_ref();
            if let Some(p) = prefix_of(id) {
                map.entry(p).or_default().push(id.to_string());
            }
        }
        PrefixIndex(map)
    }

    /// 이 세션의 접두어가 자기 자신에게만 대응되는가.
    pub fn is_unique(&self, session_id: &str) -> bool {
        match prefix_of(session_id) {
            Some(p) => self.0.get(&p).map(|v| v.len() == 1).unwrap_or(true),
            None => false,
        }
    }

    pub fn owner_of_prefix(&self, prefix: &str) -> Option<&str> {
        match self.0.get(prefix) {
            Some(v) if v.len() == 1 => Some(v[0].as_str()),
            _ => None,
        }
    }
}

pub fn prefix_of(session_id: &str) -> Option<String> {
    (session_id.len() >= PREFIX_LEN).then(|| session_id[..PREFIX_LEN].to_string())
}

/// 이 세션이 소유한 모든 파일과, 소유권이 모호한지 여부.
pub fn collect_for(
    paths: &Paths,
    session_id: &str,
    transcript: Option<&Path>,
    index: &PrefixIndex,
) -> (Vec<Artifact>, bool) {
    let mut out = Vec::new();
    let mut ambiguous = false;

    if let Some(t) = transcript {
        if let Some(a) = Artifact::at(t.to_path_buf(), ArtifactKind::Transcript) {
            out.push(a);
        }
        // projects/<p>/<uuid>/ — subagents 등이 들어있는 세션 폴더.
        if let Some(dir) = t.parent().map(|d| d.join(session_id))
            && let Some(a) = Artifact::at(dir, ArtifactKind::SessionDir)
        {
            out.push(a);
        }
    }

    // 전체 UUID로 이름이 정해지는 것들 — 소유가 확실하다.
    for (dir, kind) in [
        ("session-env", ArtifactKind::SessionEnv),
        ("file-history", ArtifactKind::FileHistory),
    ] {
        if let Some(a) = Artifact::at(paths.sidecar_dir(dir).join(session_id), kind) {
            out.push(a);
        }
    }

    // todos/ 는 이 설치에는 없지만 버전에 따라 존재한다. `<uuid>*` 형태를 모두 잡는다.
    for p in fsutil::list_dir(&paths.sidecar_dir("todos")) {
        if file_name_of(&p).starts_with(session_id)
            && let Some(a) = Artifact::at(p, ArtifactKind::Todo)
        {
            out.push(a);
        }
    }

    // 8자 접두어 키 — 유일할 때만 소유로 인정한다.
    let unique = index.is_unique(session_id);
    if let Some(prefix) = prefix_of(session_id) {
        let key = format!("{SESSION_PREFIX}{prefix}");
        for (dir, kind) in [("tasks", ArtifactKind::Task), ("teams", ArtifactKind::Team)] {
            let candidate = paths.sidecar_dir(dir).join(&key);
            if std::fs::symlink_metadata(&candidate).is_ok() {
                if unique {
                    if let Some(a) = Artifact::at(candidate, kind) {
                        out.push(a);
                    }
                } else {
                    ambiguous = true;
                }
            }
        }
    }

    // debug/ 는 이름에 전체 UUID를 포함한 것만.
    for p in fsutil::list_dir(&paths.sidecar_dir("debug")) {
        if file_name_of(&p).contains(session_id)
            && let Some(a) = Artifact::at(p, ArtifactKind::Debug)
        {
            out.push(a);
        }
    }

    (out, ambiguous)
}

/// R5: 대화 기록은 없는데 연결 데이터만 남은 세션 ID들.
pub fn orphan_session_ids(paths: &Paths, known: &HashSet<String>) -> Vec<String> {
    let mut found: HashSet<String> = HashSet::new();

    for dir in ["session-env", "file-history", "todos"] {
        for p in fsutil::list_dir(&paths.sidecar_dir(dir)) {
            let name = file_name_of(&p);
            // todos/ 는 `<uuid>-agent-<uuid>.json` 같은 형태도 있으므로 앞부분만 본다.
            let id = name.split_once('.').map_or(name.as_str(), |(a, _)| a);
            let id = id.split_once("-agent-").map_or(id, |(a, _)| a);
            if looks_like_uuid(id) && !known.contains(id) {
                found.insert(id.to_string());
            }
        }
    }

    // tasks/·teams/ 는 접두어뿐이라 세션 ID를 복원할 수 없다.
    // 알려진 어떤 세션의 접두어와도 맞지 않을 때만 고아로 본다.
    let known_prefixes: HashSet<String> = known.iter().filter_map(|id| prefix_of(id)).collect();
    for dir in ["tasks", "teams"] {
        for p in fsutil::list_dir(&paths.sidecar_dir(dir)) {
            let name = file_name_of(&p);
            let Some(prefix) = name.strip_prefix(SESSION_PREFIX) else {
                continue;
            };
            if prefix.len() == PREFIX_LEN
                && !known_prefixes.contains(prefix)
                && !found.iter().any(|id| id.starts_with(prefix))
            {
                found.insert(format!("{SESSION_PREFIX}{prefix}"));
            }
        }
    }

    let mut out: Vec<String> = found.into_iter().collect();
    out.sort();
    out
}

/// 고아 항목(`session-<prefix>` 형태 포함)의 연결 파일을 찾는다.
pub fn collect_orphan(paths: &Paths, key: &str) -> Vec<Artifact> {
    if let Some(prefix) = key.strip_prefix(SESSION_PREFIX)
        && !looks_like_uuid(key)
    {
        let mut out = Vec::new();
        for (dir, kind) in [("tasks", ArtifactKind::Task), ("teams", ArtifactKind::Team)] {
            let p = paths
                .sidecar_dir(dir)
                .join(format!("{SESSION_PREFIX}{prefix}"));
            if let Some(a) = Artifact::at(p, kind) {
                out.push(a);
            }
        }
        return out;
    }
    let index = PrefixIndex::build([key]);
    collect_for(paths, key, None, &index).0
}

pub fn file_name_of(p: &Path) -> String {
    p.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// 8-4-4-4-12 형태의 16진수인가. 엄격하게 볼수록 오탐이 줄어든다.
pub fn looks_like_uuid(s: &str) -> bool {
    let groups = [8usize, 4, 4, 4, 12];
    let parts: Vec<&str> = s.split('-').collect();
    parts.len() == groups.len()
        && parts
            .iter()
            .zip(groups)
            .all(|(p, n)| p.len() == n && p.chars().all(|c| c.is_ascii_hexdigit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "aaaaaaaa-1111-2222-3333-444444444444";
    const B: &str = "aaaaaaaa-9999-8888-7777-666666666666"; // A와 같은 접두어
    const C: &str = "cccccccc-1111-2222-3333-444444444444";

    fn setup() -> (tempfile::TempDir, Paths) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::with_roots(tmp.path().join("claude"), tmp.path().join("data"));
        std::fs::create_dir_all(paths.projects_dir()).unwrap();
        (tmp, paths)
    }

    fn touch_dir(p: PathBuf) -> PathBuf {
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(p.join("f"), b"x").unwrap();
        p
    }

    #[test]
    fn uuid_shape_is_checked_strictly() {
        assert!(looks_like_uuid(A));
        assert!(!looks_like_uuid("session-aaaaaaaa"));
        assert!(!looks_like_uuid("aaaaaaaa-1111-2222-3333-44444444444z"));
    }

    #[test]
    fn collects_full_uuid_artifacts() {
        let (_t, paths) = setup();
        let proj = paths.projects_dir().join("-w");
        std::fs::create_dir_all(&proj).unwrap();
        let transcript = proj.join(format!("{A}.jsonl"));
        std::fs::write(&transcript, b"{}").unwrap();
        touch_dir(proj.join(A).join("subagents"));
        touch_dir(paths.sidecar_dir("session-env").join(A));
        touch_dir(paths.sidecar_dir("file-history").join(A));

        let index = PrefixIndex::build([A]);
        let (arts, ambiguous) = collect_for(&paths, A, Some(&transcript), &index);
        assert!(!ambiguous);
        let kinds: HashSet<_> = arts.iter().map(|a| a.kind).collect();
        assert!(kinds.contains(&ArtifactKind::Transcript));
        assert!(kinds.contains(&ArtifactKind::SessionDir));
        assert!(kinds.contains(&ArtifactKind::SessionEnv));
        assert!(kinds.contains(&ArtifactKind::FileHistory));
    }

    #[test]
    fn prefix_collision_marks_ownership_ambiguous_and_skips_those_dirs() {
        let (_t, paths) = setup();
        touch_dir(paths.sidecar_dir("tasks").join("session-aaaaaaaa"));
        touch_dir(paths.sidecar_dir("teams").join("session-aaaaaaaa"));

        let index = PrefixIndex::build([A, B]);
        assert!(!index.is_unique(A));
        let (arts, ambiguous) = collect_for(&paths, A, None, &index);
        assert!(ambiguous, "접두어가 겹치면 소유를 확정할 수 없다");
        assert!(
            !arts.iter().any(|a| a.kind == ArtifactKind::Task),
            "모호하면 포함하지 않는다"
        );
    }

    #[test]
    fn unique_prefix_owns_task_and_team_dirs() {
        let (_t, paths) = setup();
        touch_dir(paths.sidecar_dir("tasks").join("session-cccccccc"));
        touch_dir(paths.sidecar_dir("teams").join("session-cccccccc"));
        let index = PrefixIndex::build([A, C]);
        let (arts, ambiguous) = collect_for(&paths, C, None, &index);
        assert!(!ambiguous);
        assert_eq!(arts.len(), 2);
    }

    #[test]
    fn missing_directories_are_not_errors() {
        let (_t, paths) = setup();
        let index = PrefixIndex::build([A]);
        let (arts, ambiguous) = collect_for(&paths, A, None, &index);
        assert!(arts.is_empty());
        assert!(!ambiguous);
    }

    #[test]
    fn debug_entries_require_the_full_uuid() {
        let (_t, paths) = setup();
        let debug = paths.sidecar_dir("debug");
        std::fs::create_dir_all(&debug).unwrap();
        std::fs::write(debug.join(format!("{A}.log")), b"x").unwrap();
        std::fs::write(debug.join("latest.log"), b"x").unwrap();
        let index = PrefixIndex::build([A]);
        let (arts, _) = collect_for(&paths, A, None, &index);
        assert_eq!(arts.len(), 1);
        assert_eq!(arts[0].kind, ArtifactKind::Debug);
    }

    #[test]
    fn finds_orphan_ids_without_transcripts() {
        let (_t, paths) = setup();
        touch_dir(paths.sidecar_dir("session-env").join(A));
        touch_dir(paths.sidecar_dir("file-history").join(C));
        touch_dir(paths.sidecar_dir("tasks").join("session-eeeeeeee"));

        let known: HashSet<String> = [A.to_string()].into_iter().collect();
        let orphans = orphan_session_ids(&paths, &known);
        assert!(!orphans.contains(&A.to_string()), "알려진 세션은 고아가 아니다");
        assert!(orphans.contains(&C.to_string()));
        assert!(orphans.contains(&"session-eeeeeeee".to_string()));
    }

    #[test]
    fn task_dir_of_a_known_session_is_not_reported_as_orphan() {
        let (_t, paths) = setup();
        touch_dir(paths.sidecar_dir("tasks").join("session-aaaaaaaa"));
        let known: HashSet<String> = [A.to_string()].into_iter().collect();
        assert!(orphan_session_ids(&paths, &known).is_empty());
    }

    #[test]
    fn fingerprint_changes_when_content_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("a");
        std::fs::write(&f, b"one").unwrap();
        let before = Fingerprint::of(&f).unwrap();
        std::fs::write(&f, b"one and more").unwrap();
        assert_ne!(before, Fingerprint::of(&f).unwrap());
    }

    #[test]
    fn fingerprint_of_directory_tracks_contents() {
        let tmp = tempfile::tempdir().unwrap();
        let d = touch_dir(tmp.path().join("d"));
        let before = Fingerprint::of(&d).unwrap();
        std::fs::write(d.join("g"), b"more bytes here").unwrap();
        assert_ne!(before, Fingerprint::of(&d).unwrap());
    }
}
