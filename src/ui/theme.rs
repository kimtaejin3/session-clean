//! 화면 기호와 색.
//!
//! PRD §8.5 / §15: "색상은 상태 구분을 보조할 뿐 유일한 표현 수단으로 사용하지
//! 않는다." 모든 상태에는 반드시 대응하는 **기호나 낱말**이 있고, 색은 그 위에만 얹는다.

use ratatui::style::Color;

pub const SEL_ON: &str = "[x]";
pub const SEL_OFF: &str = "[ ]";
pub const SEL_BLOCKED: &str = "[-]";
pub const MARK_RECOMMENDED: &str = "★";
pub const MARK_PLAIN: &str = " ";
pub const MARK_UNPARSABLE: &str = "!";
pub const MARK_RUNNING: &str = "▶";
pub const EXPANDED: &str = "▾";
pub const COLLAPSED: &str = "▸";

pub const ACCENT: Color = Color::Cyan;
pub const RECOMMEND: Color = Color::Yellow;
pub const DANGER: Color = Color::Red;
pub const MUTED: Color = Color::DarkGray;
pub const OK: Color = Color::Green;

/// 최소 사용 가능한 터미널 크기 (PRD §14).
pub const MIN_WIDTH: u16 = 50;
pub const MIN_HEIGHT: u16 = 12;
/// 이 폭 미만이면 크기 열을 접는다 (FR-20).
pub const WIDE_ENOUGH: u16 = 100;
/// 이 폭 미만이면 프로젝트 패널을 접고 한 줄 헤더로 바꾼다.
pub const TWO_PANE: u16 = 72;

/// `92일 전`, `3시간 전`, `방금`.
pub fn relative_time(now_secs: i64, then_secs: i64) -> String {
    let d = now_secs.saturating_sub(then_secs).max(0);
    match d {
        0..=59 => "방금".to_string(),
        60..=3599 => format!("{}분 전", d / 60),
        3600..=86_399 => format!("{}시간 전", d / 3600),
        _ => format!("{}일 전", d / 86_400),
    }
}

/// 폭에 맞춰 문자열을 자른다. 한글은 두 칸을 차지하므로 폭 기준으로 센다.
pub fn fit(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0usize;
    for ch in text.chars() {
        let w = char_width(ch);
        if used + w > width.saturating_sub(1) && display_width(text) > width {
            out.push('…');
            return out;
        }
        out.push(ch);
        used += w;
    }
    out
}

pub fn pad(text: &str, width: usize) -> String {
    let mut s = fit(text, width);
    let w = display_width(&s);
    if w < width {
        s.push_str(&" ".repeat(width - w));
    }
    s
}

pub fn display_width(text: &str) -> usize {
    text.chars().map(char_width).sum()
}

/// 한글·한자·가나·전각 기호는 두 칸.
fn char_width(c: char) -> usize {
    let cp = c as u32;
    let wide = (0x1100..=0x115F).contains(&cp)
        || (0x2E80..=0xA4CF).contains(&cp)
        || (0xAC00..=0xD7A3).contains(&cp)
        || (0xF900..=0xFAFF).contains(&cp)
        || (0xFE30..=0xFE6F).contains(&cp)
        || (0xFF00..=0xFF60).contains(&cp)
        || (0xFFE0..=0xFFE6).contains(&cp);
    if wide { 2 } else { 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_time_reads_naturally() {
        assert_eq!(relative_time(1000, 1000), "방금");
        assert_eq!(relative_time(10_000, 10_000 - 120), "2분 전");
        assert_eq!(relative_time(100_000, 100_000 - 7200), "2시간 전");
        assert_eq!(
            relative_time(10_000_000, 10_000_000 - 92 * 86_400),
            "92일 전"
        );
    }

    #[test]
    fn wide_characters_count_as_two_columns() {
        assert_eq!(display_width("로그인"), 6);
        assert_eq!(display_width("abc"), 3);
        assert_eq!(pad("로그인", 10), "로그인    ");
    }

    #[test]
    fn fit_truncates_with_an_ellipsis() {
        assert_eq!(fit("abcdefgh", 4), "abc…");
        assert_eq!(fit("abc", 10), "abc");
    }

    #[test]
    fn every_state_has_a_symbol_not_only_a_color() {
        // 선택·추천·차단 상태는 색 없이도 구분된다.
        let symbols = [
            SEL_ON,
            SEL_OFF,
            SEL_BLOCKED,
            MARK_RECOMMENDED,
            MARK_UNPARSABLE,
            MARK_RUNNING,
        ];
        assert_eq!(
            symbols
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            symbols.len()
        );
    }
}
