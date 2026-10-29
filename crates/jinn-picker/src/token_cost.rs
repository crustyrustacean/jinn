//! Compact token-count formatting for picker status lines.
//!
//! One formatter shared by every picker's context-cost figure so the numbers
//! read the same way the status bar reads them: `999` stays as-is, larger
//! counts compact to `1.2k` / `3.4M`.

/// Formats a token count compactly, matching the status bar's convention.
///
/// `999` → `"999"`, `1234` → `"1.2k"`, `3_400_000` → `"3.4M"`.
#[must_use]
pub fn format_compact(count: usize) -> String {
    if count >= 1_000_000 {
        format!("{:.1}M", count as f64 / 1_000_000.0)
    } else if count >= 1_000 {
        format!("{:.1}k", count as f64 / 1_000.0)
    } else {
        count.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[rstest::rstest]
    #[case(0, "0")]
    #[case(999, "999")]
    #[case(1_000, "1.0k")]
    #[case(1_234, "1.2k")]
    #[case(45_600, "45.6k")]
    #[case(3_400_000, "3.4M")]
    #[case(999_999_999, "1000.0M")]
    fn compact_format_follows_the_status_bar_convention(
        #[case] count: usize,
        #[case] expected: &str,
    ) {
        // Given a token count.
        // When formatting it compactly.
        let formatted = format_compact(count);
        // Then it matches the status bar's convention.
        assert_eq!(formatted, expected);
    }
}
