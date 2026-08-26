//! PRD §15 성능 / §16 수동 시나리오 4:
//! "일반적인 Apple Silicon Mac에서 2,000개 세션을 3초 안에 첫 화면에 표시한다."
//!
//! 디버그 빌드는 릴리스보다 훨씬 느리므로 여기서는 넉넉한 상한을 쓴다.
//! 실제 수치는 `cargo test --release --test perf_test -- --nocapture` 로 확인한다.

mod support;

use sclean::scan::scan;
use support::Fixture;

const SESSIONS: usize = 2_000;
const PROJECTS: usize = 20;

#[test]
fn scans_two_thousand_sessions_quickly() {
    let f = Fixture::new();
    let mut roots = Vec::new();
    for p in 0..PROJECTS {
        roots.push(f.source_tree(&format!("project-{p}")));
    }

    for i in 0..SESSIONS {
        let root = &roots[i % PROJECTS];
        let id = format!("{i:08x}-1111-2222-3333-444444444444");
        let mut b = f
            .session(root.to_str().unwrap(), &id)
            .summary(&format!("작업 {i}"))
            .user("첫 질문")
            .tool_use("Edit")
            .assistant("답변");
        // 현실적인 분포: 일부 세션에는 연결 데이터가 붙어 있다.
        if i % 5 == 0 {
            b = b.with_env();
        }
        if i % 11 == 0 {
            b = b.with_task();
        }
        b.age_days((i % 200) as i64).build();
    }

    let start = std::time::Instant::now();
    let result = scan(&f.paths());
    let elapsed = start.elapsed();

    assert_eq!(result.session_count(), SESSIONS);
    assert_eq!(result.projects.len(), PROJECTS);
    println!(
        "스캔 {SESSIONS}개 세션: {:?} ({:.1} 세션/ms)",
        elapsed,
        SESSIONS as f64 / elapsed.as_millis().max(1) as f64
    );

    let budget = if cfg!(debug_assertions) { 9 } else { 3 };
    assert!(
        elapsed.as_secs() < budget,
        "{SESSIONS}개 스캔에 {elapsed:?} 걸렸습니다 (상한 {budget}초)"
    );
}

#[test]
fn a_huge_transcript_does_not_get_fully_loaded() {
    // PRD §15: "세션 전체 내용을 메모리에 계속 보관하지 않는다."
    // 아주 큰 세션도 판정에 필요한 앞부분만 읽고 끝나야 한다.
    let f = Fixture::new();
    let p = f.source_tree("huge");
    let id = "ffffffff-1111-2222-3333-444444444444";
    let mut b = f
        .session(p.to_str().unwrap(), id)
        .user("q1")
        .tool_use("Bash");
    for i in 0..20_000 {
        b = b.assistant(&format!("아주 긴 답변 {i} {}", "가".repeat(200)));
    }
    b.age_days(90).build();

    let start = std::time::Instant::now();
    let result = scan(&f.paths());
    let elapsed = start.elapsed();

    assert_eq!(result.session_count(), 1);
    let s = result.sessions().next().unwrap();
    assert!(s.size_bytes > 5_000_000, "충분히 큰 파일이어야 한다");
    println!("거대 세션 스캔: {elapsed:?} ({} bytes)", s.size_bytes);
    assert!(
        elapsed.as_millis() < 500,
        "조기 중단이 동작하지 않습니다: {elapsed:?}"
    );
}
