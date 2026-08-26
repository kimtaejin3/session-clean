//! 파일 시스템 원시 연산과 경로 가드.
//!
//! 여기 있는 모든 함수는 심볼릭 링크를 **따라가지 않는다**. 링크를 따라가면
//! `~/.claude` 밖의 파일을 건드릴 수 있고, 그것은 FR-16 위반이다.

use anyhow::{Context, Result, bail};
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// 같은 디렉터리에 임시 파일을 쓴 뒤 rename으로 교체한다.
///
/// PRD §15 안정성: "공유 파일 변경은 임시 파일과 원자적 교체를 사용한다."
/// 같은 디렉터리를 쓰는 이유는 rename이 같은 파일시스템 안에서만 원자적이기 때문.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "out".into());
    let tmp = dir.join(format!(".{name}.sclean-tmp-{}-{n}", std::process::id()));

    let result = (|| -> io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        io::Write::write_all(&mut f, bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// 파일 또는 디렉터리를 옮긴다. 같은 볼륨이면 rename(원자적),
/// 다른 볼륨이면 복사 후 원본 삭제로 대체한다.
pub fn move_path(from: &Path, to: &Path) -> io::Result<()> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
            copy_tree(from, to)?;
            remove_path(from)
        }
        Err(e) => Err(e),
    }
}

/// 심볼릭 링크는 링크 자체로 재생성하고, 절대 따라가지 않는다.
pub fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    let meta = std::fs::symlink_metadata(from)?;
    if meta.file_type().is_symlink() {
        let target = std::fs::read_link(from)?;
        return std::os::unix::fs::symlink(target, to);
    }
    if meta.is_dir() {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        }
        return Ok(());
    }
    std::fs::copy(from, to).map(|_| ())
}

/// 파일·디렉터리·심볼릭 링크를 모두 지운다. 링크는 링크만 지운다.
pub fn remove_path(path: &Path) -> io::Result<()> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        // 이미 없으면 성공으로 본다 — 삭제/복원은 멱등해야 한다(PRD §15).
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    if meta.file_type().is_symlink() || meta.is_file() {
        std::fs::remove_file(path)
    } else {
        std::fs::remove_dir_all(path)
    }
}

/// 디렉터리면 재귀 합계, 파일이면 파일 크기. 링크는 따라가지 않고 0으로 센다.
pub fn entry_size(path: &Path) -> u64 {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if meta.file_type().is_symlink() {
        return 0;
    }
    if meta.is_file() {
        return meta.len();
    }
    let Ok(rd) = std::fs::read_dir(path) else {
        return 0;
    };
    rd.flatten().map(|e| entry_size(&e.path())).sum()
}

/// FR-16 / PRD §12-3: 대상이 정말 `root` 안에 있는지 검증한다.
///
/// 대상의 **부모**만 canonicalize한다. 대상 자체를 canonicalize하면 대상이
/// 심볼릭 링크일 때 링크가 가리키는 바깥 경로를 검사하게 되어 의미가 뒤집힌다.
/// 대상이 심볼릭 링크면 아예 거부한다 — Claude 데이터에 링크가 있을 이유가
/// 없고, 불확실한 것은 건드리지 않는 것이 안전 원칙이다(PRD §6).
pub fn ensure_within(root: &Path, target: &Path) -> Result<()> {
    if target
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        bail!("경로에 상위 참조(..)가 있습니다: {}", target.display());
    }
    let root_c = std::fs::canonicalize(root)
        .with_context(|| format!("기준 경로를 확인할 수 없습니다: {}", root.display()))?;
    let parent = target
        .parent()
        .ok_or_else(|| anyhow::anyhow!("경로에 부모가 없습니다: {}", target.display()))?;
    let parent_c = std::fs::canonicalize(parent)
        .with_context(|| format!("상위 경로를 확인할 수 없습니다: {}", parent.display()))?;
    let name = target
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("경로에 이름이 없습니다: {}", target.display()))?;
    let full = parent_c.join(name);

    if full == root_c {
        bail!(
            "데이터 루트 자체는 대상이 될 수 없습니다: {}",
            full.display()
        );
    }
    if !full.starts_with(&root_c) {
        bail!(
            "{} 은(는) {} 밖에 있습니다",
            full.display(),
            root_c.display()
        );
    }
    if let Ok(meta) = std::fs::symlink_metadata(&full)
        && meta.file_type().is_symlink()
    {
        bail!("심볼릭 링크는 정리 대상이 아닙니다: {}", full.display());
    }
    Ok(())
}

/// 사람이 읽는 크기 표기. 색 없이도 구분 가능해야 하므로 단위를 항상 붙인다.
pub fn human_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.0} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

/// 디렉터리 하위의 최상위 항목 이름들. 없으면 빈 목록(오류 아님).
pub fn list_dir(path: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(path) else {
        return Vec::new();
    };
    rd.flatten().map(|e| e.path()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_content() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("nested/a.json");
        atomic_write(&f, b"one").unwrap();
        atomic_write(&f, b"two").unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), b"two");
        // 임시 파일이 남지 않아야 한다.
        let leftovers: Vec<_> = list_dir(f.parent().unwrap())
            .into_iter()
            .filter(|p| p.to_string_lossy().contains("sclean-tmp"))
            .collect();
        assert!(leftovers.is_empty(), "임시 파일 잔여: {leftovers:?}");
    }

    #[test]
    fn move_path_moves_files_and_directories() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("src/d");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("x"), b"hi").unwrap();
        let dst = tmp.path().join("dst/d");
        move_path(&dir, &dst).unwrap();
        assert!(!dir.exists());
        assert_eq!(std::fs::read(dst.join("x")).unwrap(), b"hi");
    }

    #[test]
    fn remove_path_on_missing_target_is_ok() {
        let tmp = tempfile::tempdir().unwrap();
        remove_path(&tmp.path().join("nope")).unwrap();
    }

    #[test]
    fn remove_path_deletes_symlink_not_its_target() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("real.txt");
        std::fs::write(&real, b"keep").unwrap();
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        remove_path(&link).unwrap();
        assert!(!link.exists());
        assert!(real.exists(), "링크 대상은 살아있어야 한다");
    }

    #[test]
    fn ensure_within_accepts_real_children() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("claude");
        std::fs::create_dir_all(root.join("projects/p")).unwrap();
        let target = root.join("projects/p/s.jsonl");
        std::fs::write(&target, b"{}").unwrap();
        ensure_within(&root, &target).unwrap();
    }

    #[test]
    fn ensure_within_rejects_escape_via_parent_components() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("claude");
        std::fs::create_dir_all(&root).unwrap();
        let outside = tmp.path().join("outside.txt");
        std::fs::write(&outside, b"x").unwrap();
        let sneaky = root.join("../outside.txt");
        assert!(ensure_within(&root, &sneaky).is_err());
    }

    #[test]
    fn ensure_within_rejects_paths_under_a_symlinked_parent() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("claude");
        std::fs::create_dir_all(&root).unwrap();
        let outside = tmp.path().join("elsewhere");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("f.txt"), b"x").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("linkdir")).unwrap();
        // 부모를 canonicalize 하므로 링크 밖으로 나간 것이 드러난다.
        assert!(ensure_within(&root, &root.join("linkdir/f.txt")).is_err());
    }

    #[test]
    fn ensure_within_rejects_symlink_targets() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("claude");
        std::fs::create_dir_all(&root).unwrap();
        let outside = tmp.path().join("outside.txt");
        std::fs::write(&outside, b"x").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
        assert!(ensure_within(&root, &root.join("link")).is_err());
    }

    #[test]
    fn ensure_within_rejects_root_itself() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("claude");
        std::fs::create_dir_all(&root).unwrap();
        assert!(ensure_within(&root, &root).is_err());
    }

    #[test]
    fn entry_size_sums_directory_recursively_without_following_links() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path().join("d/sub");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("a"), vec![0u8; 100]).unwrap();
        std::fs::write(tmp.path().join("d/b"), vec![0u8; 50]).unwrap();
        let big = tmp.path().join("big");
        std::fs::write(&big, vec![0u8; 10_000]).unwrap();
        std::os::unix::fs::symlink(&big, tmp.path().join("d/link")).unwrap();
        assert_eq!(entry_size(&tmp.path().join("d")), 150);
    }

    #[test]
    fn human_bytes_always_carries_a_unit() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(2048), "2 KB");
        assert_eq!(human_bytes(3 * 1024 * 1024), "3.0 MB");
    }
}
