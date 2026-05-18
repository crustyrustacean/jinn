//! Shared truncation utilities for tool outputs.
//
//! Truncation is based on two independent limits — whichever is hit first wins:
//! - Line limit (default: 2000 lines)
//! - Byte limit (default: 50KB)
//!
//! Two strategies are provided:
//! - [`truncate_head`] — keeps the beginning of content (for `read`)
//! - [`truncate_tail`] — keeps the end of content (for `bash`)
//!
//! Both return a [`TruncationResult`] carrying the truncated content and
//! optional metadata about what was removed.

// Re-export truncation types from the provider crate so domain code can
// reference them without depending on the provider directly.
pub use nullslop_provider::tool_types::{TruncatedBy, TruncationMeta};

/// Default maximum number of output lines.
pub const DEFAULT_MAX_LINES: usize = 2000;

/// Default maximum output size in bytes.
pub const DEFAULT_MAX_BYTES: usize = 50 * 1024; // 50KB

/// Result of a truncation operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TruncationResult {
    /// The truncated content (or original if not truncated).
    pub content: String,
    /// Whether truncation occurred.
    pub truncated: bool,
    /// Truncation metadata (present only when truncated).
    pub meta: Option<TruncationMeta>,
    /// The max lines limit that was applied.
    pub max_lines: usize,
    /// The max bytes limit that was applied.
    pub max_bytes: usize,
}

/// Truncate content from the head (keep first N lines/bytes).
///
/// Suitable for file reads where the beginning of the content is most
/// useful (headers, imports, structure).
///
/// Never returns partial lines. If the first line alone exceeds the byte
/// limit, returns empty content.
pub fn truncate_head(content: &str, max_lines: usize, max_bytes: usize) -> TruncationResult {
    let total_bytes = content.len();
    let lines: Vec<&str> = content.split('\n').collect();
    let total_lines = lines.len();

    // Check if no truncation is needed.
    if total_lines <= max_lines && total_bytes <= max_bytes {
        return TruncationResult {
            content: content.to_owned(),
            truncated: false,
            meta: None,
            max_lines,
            max_bytes,
        };
    }

    // If the first line alone exceeds the byte limit, return empty.
    if !lines.is_empty() && lines[0].len() > max_bytes {
        return TruncationResult {
            content: String::new(),
            truncated: true,
            meta: Some(TruncationMeta {
                truncated_by: TruncatedBy::Bytes,
                total_lines,
                total_bytes,
                output_lines: 0,
                output_bytes: 0,
            }),
            max_lines,
            max_bytes,
        };
    }

    // Collect complete lines that fit within both limits.
    let mut output_lines_arr: Vec<&str> = Vec::new();
    let mut output_bytes_count: usize = 0;
    let mut truncated_by = TruncatedBy::Lines;

    for (i, line) in lines.iter().enumerate() {
        // +1 for newline separator (except the first line).
        let line_bytes = line.len() + usize::from(i > 0);

        if output_bytes_count + line_bytes > max_bytes {
            truncated_by = TruncatedBy::Bytes;
            break;
        }

        if output_lines_arr.len() >= max_lines {
            truncated_by = TruncatedBy::Lines;
            break;
        }

        output_lines_arr.push(line);
        output_bytes_count += line_bytes;
    }

    // If we collected exactly max_lines and still within bytes, it's a line truncation.
    if output_lines_arr.len() >= max_lines && output_bytes_count <= max_bytes {
        truncated_by = TruncatedBy::Lines;
    }

    let output_content = output_lines_arr.join("\n");
    let output_bytes = output_content.len();

    TruncationResult {
        content: output_content,
        truncated: true,
        meta: Some(TruncationMeta {
            truncated_by,
            total_lines,
            total_bytes,
            output_lines: output_lines_arr.len(),
            output_bytes,
        }),
        max_lines,
        max_bytes,
    }
}

/// Truncate content from the tail (keep last N lines/bytes).
///
/// Suitable for command output where errors and final results appear at
/// the end.
///
/// May return a partial first line if the last line alone exceeds the
/// byte limit (edge case: single enormous line).
pub fn truncate_tail(content: &str, max_lines: usize, max_bytes: usize) -> TruncationResult {
    let total_bytes = content.len();
    let lines: Vec<&str> = content.split('\n').collect();
    let total_lines = lines.len();

    // Check if no truncation is needed.
    if total_lines <= max_lines && total_bytes <= max_bytes {
        return TruncationResult {
            content: content.to_owned(),
            truncated: false,
            meta: None,
            max_lines,
            max_bytes,
        };
    }

    // Work backwards from the end.
    let mut output_lines_arr: Vec<&str> = Vec::new();
    let mut output_bytes_count: usize = 0;
    let mut truncated_by = TruncatedBy::Lines;
    let mut last_line_partial = false;

    for line in lines.iter().rev() {
        // +1 for newline separator (except when this is the first line added).
        let line_bytes = line.len() + usize::from(!output_lines_arr.is_empty());

        if output_bytes_count + line_bytes > max_bytes {
            truncated_by = TruncatedBy::Bytes;
            // Edge case: no lines added yet and this line exceeds the limit.
            // Take the tail of the line (partial).
            if output_lines_arr.is_empty() && !line.is_empty() {
                let truncated_line = truncate_string_from_end(line, max_bytes);
                output_lines_arr.insert(0, ""); // Will be replaced
                output_bytes_count = truncated_line.len();
                last_line_partial = true;
                // We insert the truncated line after reversing.
            }
            break;
        }

        if output_lines_arr.len() >= max_lines {
            truncated_by = TruncatedBy::Lines;
            break;
        }

        output_lines_arr.push(line);
        output_bytes_count += line_bytes;
    }

    // Reverse to restore original order.
    output_lines_arr.reverse();

    // If we collected exactly max_lines and still within bytes, it's a line truncation.
    if output_lines_arr.len() >= max_lines && output_bytes_count <= max_bytes {
        truncated_by = TruncatedBy::Lines;
    }

    let output_content = if last_line_partial {
        // Reconstruct with the partial first line.
        let partial = truncate_string_from_end(lines.last().unwrap_or(&""), max_bytes);
        if output_lines_arr.len() > 1 {
            format!("{}\n{}", partial, output_lines_arr[1..].join("\n"))
        } else {
            partial
        }
    } else {
        output_lines_arr.join("\n")
    };

    let output_bytes = output_content.len();

    TruncationResult {
        content: output_content,
        truncated: true,
        meta: Some(TruncationMeta {
            truncated_by,
            total_lines,
            total_bytes,
            output_lines: output_lines_arr.len(),
            output_bytes,
        }),
        max_lines,
        max_bytes,
    }
}

/// Truncate a string to fit within a byte limit, keeping the end.
///
/// Handles multi-byte UTF-8 characters correctly by finding a valid
/// char boundary.
fn truncate_string_from_end(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_owned();
    }

    // Start from max_bytes back, find a valid char boundary.
    let start = max_bytes;
    let mut boundary = start;
    while boundary < s.len() && !s.is_char_boundary(boundary) {
        boundary += 1;
    }
    s[boundary..].to_owned()
}

/// Format a byte count as a human-readable size string.
#[allow(clippy::cast_precision_loss)]
pub fn format_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes}B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1}KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1}MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- truncate_head ---

    #[rstest::rstest]
    fn head_no_truncation_when_under_limits() {
        // Given content well within both limits.
        let content = "line1\nline2\nline3";

        // When truncating with generous limits.
        let result = truncate_head(content, 100, 1024);

        // Then content passes through unchanged.
        assert!(!result.truncated);
        assert_eq!(result.content, content);
        assert!(result.meta.is_none());
    }

    #[rstest::rstest]
    fn head_truncates_by_lines() {
        // Given 5 lines of content and a 3-line limit.
        let content = "line1\nline2\nline3\nline4\nline5";

        // When truncating with 3-line limit.
        let result = truncate_head(content, 3, 1024);

        // Then only the first 3 lines are kept.
        assert!(result.truncated);
        assert_eq!(result.content, "line1\nline2\nline3");
        let meta = result.meta.unwrap();
        assert_eq!(meta.truncated_by, TruncatedBy::Lines);
        assert_eq!(meta.total_lines, 5);
        assert_eq!(meta.output_lines, 3);
    }

    #[rstest::rstest]
    fn head_truncates_by_bytes() {
        // Given 4 lines of 6 bytes each. With a 14-byte limit:
        // line 0: "123456" = 6 bytes, total 6
        // line 1: "\n123456" = 7 bytes, total 13
        // line 2: "\n123456" = 7 bytes, total 20 > 14, stop.
        let content = "123456\n123456\n123456\n123456";

        // When truncating with a 14-byte limit.
        let result = truncate_head(content, 100, 14);

        // Then only the first 2 lines are kept (13 bytes).
        assert!(result.truncated);
        assert_eq!(result.content, "123456\n123456");
        let meta = result.meta.unwrap();
        assert_eq!(meta.truncated_by, TruncatedBy::Bytes);
    }

    #[rstest::rstest]
    fn head_returns_empty_when_first_line_exceeds_byte_limit() {
        // Given a single line larger than the byte limit.
        let content = "a".repeat(200);

        // When truncating with a 100-byte limit.
        let result = truncate_head(&content, 100, 100);

        // Then content is empty.
        assert!(result.truncated);
        assert!(result.content.is_empty());
        let meta = result.meta.unwrap();
        assert_eq!(meta.truncated_by, TruncatedBy::Bytes);
        assert_eq!(meta.output_lines, 0);
    }

    #[rstest::rstest]
    fn head_empty_content_passes_through() {
        // Given empty content.
        let content = "";

        // When truncating.
        let result = truncate_head(content, 100, 1024);

        // Then it passes through unchanged.
        assert!(!result.truncated);
        assert!(result.content.is_empty());
    }

    #[rstest::rstest]
    fn head_single_line_no_truncation() {
        // Given a single line under the byte limit.
        let content = "hello";

        // When truncating with 1-line limit.
        let result = truncate_head(content, 1, 1024);

        // Then it passes through unchanged.
        assert!(!result.truncated);
        assert_eq!(result.content, "hello");
    }

    // --- truncate_tail ---

    #[rstest::rstest]
    fn tail_no_truncation_when_under_limits() {
        // Given content well within both limits.
        let content = "line1\nline2\nline3";

        // When truncating with generous limits.
        let result = truncate_tail(content, 100, 1024);

        // Then content passes through unchanged.
        assert!(!result.truncated);
        assert_eq!(result.content, content);
        assert!(result.meta.is_none());
    }

    #[rstest::rstest]
    fn tail_truncates_by_lines() {
        // Given 5 lines of content and a 3-line limit.
        let content = "line1\nline2\nline3\nline4\nline5";

        // When truncating with 3-line limit.
        let result = truncate_tail(content, 3, 1024);

        // Then the last 3 lines are kept.
        assert!(result.truncated);
        assert_eq!(result.content, "line3\nline4\nline5");
        let meta = result.meta.unwrap();
        assert_eq!(meta.truncated_by, TruncatedBy::Lines);
        assert_eq!(meta.total_lines, 5);
        assert_eq!(meta.output_lines, 3);
    }

    #[rstest::rstest]
    fn tail_truncates_by_bytes() {
        // Given 4 lines of 6 bytes each, with a 16-byte limit.
        // Last 2 lines (12 bytes + 1 newline = 13 bytes) should fit.
        let content = "123456\n123456\n123456\n123456";

        // When truncating with a 14-byte limit.
        let result = truncate_tail(content, 100, 14);

        // Then only the tail that fits is kept.
        assert!(result.truncated);
        let meta = result.meta.unwrap();
        assert_eq!(meta.truncated_by, TruncatedBy::Bytes);
    }

    #[rstest::rstest]
    fn tail_empty_content_passes_through() {
        // Given empty content.
        let content = "";

        // When truncating.
        let result = truncate_tail(content, 100, 1024);

        // Then it passes through unchanged.
        assert!(!result.truncated);
        assert!(result.content.is_empty());
    }

    #[rstest::rstest]
    fn tail_single_enormous_line_returns_partial() {
        // Given a single line larger than the byte limit.
        let content = "a".repeat(200);

        // When truncating with a 100-byte limit.
        let result = truncate_tail(&content, 100, 100);

        // Then a partial tail of the line is returned.
        assert!(result.truncated);
        assert!(!result.content.is_empty());
        assert!(result.content.len() <= 100);
        let meta = result.meta.unwrap();
        assert_eq!(meta.truncated_by, TruncatedBy::Bytes);
    }

    // --- format_size ---

    #[rstest::rstest]
    #[case::bytes(500, "500B")]
    #[case::kilobytes(2048, "2.0KB")]
    #[case::megabytes(2 * 1024 * 1024, "2.0MB")]
    fn format_size_formats_correctly(#[case] bytes: usize, #[case] expected: &str) {
        assert_eq!(format_size(bytes), expected);
    }
}
