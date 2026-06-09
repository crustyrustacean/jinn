//! Response formatting for the hashline edit tool.
//!
//! After a successful edit, the response includes an `--- Anchors N-M ---`
//! block with fresh LINE#HASH tags for the changed region, allowing the LLM
//! to chain follow-up edits without re-reading the file.

use super::hash::{format_hashline_region, get_visible_lines};

// ─── Constants ──────────────────────────────────────────────────────────

/// Context lines around the changed region in anchor blocks.
const ANCHOR_CONTEXT_LINES: usize = 2;

/// Maximum number of lines in an anchor block before omitting.
const ANCHOR_MAX_OUTPUT_LINES: usize = 12;

/// Byte budget for anchor blocks (50KB).
const ANCHOR_TEXT_BUDGET_BYTES: usize = 50 * 1024;

// ─── Types ──────────────────────────────────────────────────────────────

/// The affected-line range computation result.
pub struct AffectedRange {
    /// First line (1-indexed) of the anchor block.
    pub start: usize,
    /// Last line (1-indexed) of the anchor block.
    pub end: usize,
}

/// Result of computing the anchor block for a response.
#[derive(Debug)]
pub enum AnchorBlock {
    /// A formatted anchor block with fresh hashes.
    Block {
        text: String,
        #[allow(dead_code, reason = "kept for future use")]
        start: usize,
        #[allow(dead_code, reason = "kept for future use")]
        end: usize,
    },
    /// Anchors omitted because the span was too large.
    Omitted,
    /// File is empty after edit.
    EmptyFile,
}

// ─── Affected range computation ─────────────────────────────────────────

/// Computes the post-edit line range covering changed lines plus context.
///
/// Returns `None` if the range (with context) exceeds the output budget,
/// signalling that the LLM should re-read instead.
pub fn compute_affected_line_range(
    first_changed_line: Option<usize>,
    last_changed_line: Option<usize>,
    result_line_count: usize,
) -> Option<AffectedRange> {
    let first = first_changed_line?;
    let last = last_changed_line?;

    if result_line_count == 0 {
        return None;
    }

    let start = first.saturating_sub(ANCHOR_CONTEXT_LINES).max(1);
    let end = (last + ANCHOR_CONTEXT_LINES).min(result_line_count);

    if end < start {
        return None;
    }

    if end - start + 1 > ANCHOR_MAX_OUTPUT_LINES {
        return None;
    }

    Some(AffectedRange { start, end })
}

// ─── Anchor block formatting ────────────────────────────────────────────

/// Builds the anchor block for a successful edit response.
///
/// Returns the formatted text to include in the tool result.
pub fn build_anchor_block(
    result_content: &str,
    first_changed_line: Option<usize>,
    last_changed_line: Option<usize>,
) -> AnchorBlock {
    let visible = get_visible_lines(result_content);

    if visible.is_empty() {
        return AnchorBlock::EmptyFile;
    }

    let range = compute_affected_line_range(first_changed_line, last_changed_line, visible.len());

    let Some(range) = range else {
        return AnchorBlock::Omitted;
    };

    let region: Vec<&str> = visible.get(range.start.saturating_sub(1)..range.end).unwrap_or_default().to_vec();
    let formatted = format_hashline_region(&region, range.start);

    let block_text = format!("--- Anchors {}-{} ---\n{formatted}", range.start, range.end);

    if block_text.len() > ANCHOR_TEXT_BUDGET_BYTES {
        return AnchorBlock::Omitted;
    }

    AnchorBlock::Block {
        text: block_text,
        start: range.start,
        end: range.end,
    }
}

/// Formats the full response text for a successful edit.
pub fn format_success_response(
    _path: &str,
    anchor_block: &AnchorBlock,
    warnings: &[String],
) -> String {
    let anchor_text = match anchor_block {
        AnchorBlock::Block { text, .. } => text.clone(),
        AnchorBlock::Omitted => "Anchors omitted; use read for subsequent edits.".to_owned(),
        AnchorBlock::EmptyFile => {
            "File is empty. Use edit with prepend or append and omit pos to insert content."
                .to_owned()
        }
    };

    let mut parts = vec![anchor_text];

    if !warnings.is_empty() {
        let warning_text = format!("Warnings:\n{}", warnings.join("\n"));
        parts.push(warning_text);
    }

    parts.join("\n\n")
}

/// Formats the response text for a NOOP (no changes made).
pub fn format_noop_response(
    path: &str,
    noop_edits: &[super::engine::NoopEdit],
    warnings: &[String],
) -> String {
    let noop_text = if noop_edits.is_empty() {
        "The edits produced identical content.".to_owned()
    } else {
        noop_edits
            .iter()
            .map(|e| {
                format!(
                    "Edit {}: replacement for {} is identical to current content:\n  {}: {}",
                    e.edit_index, e.loc, e.loc, e.current_content
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    let mut parts = vec![format!("No changes made to {path}\n{noop_text}")];

    if !warnings.is_empty() {
        let warning_text = format!("Warnings:\n{}", warnings.join("\n"));
        parts.push(warning_text);
    }

    parts.join("\n\n")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unreachable, clippy::indexing_slicing, reason = "test code")]
    use super::*;

    #[rstest::rstest]
    fn compute_affected_range_basic() {
        // Given a changed region of lines 3-5 in a 10-line file.
        let range = compute_affected_line_range(Some(3), Some(5), 10);

        // Then the range includes 2 context lines on each side.
        let r = range.expect("should compute");
        assert_eq!(r.start, 1); // 3 - 2 = 1, clamped to 1
        assert_eq!(r.end, 7); // 5 + 2 = 7
    }

    #[rstest::rstest]
    fn compute_affected_range_clamps_to_file_bounds() {
        // Given a change at the end of a 5-line file.
        let range = compute_affected_line_range(Some(4), Some(5), 5);

        // Then end is clamped to 5.
        let r = range.expect("should compute");
        assert_eq!(r.start, 2); // 4 - 2 = 2
        assert_eq!(r.end, 5); // min(5+2, 5) = 5
    }

    #[rstest::rstest]
    fn compute_affected_range_returns_none_for_large_span() {
        // Given a change spanning 15 lines (15 + 4 context = 19 > 12).
        let range = compute_affected_line_range(Some(1), Some(15), 20);

        // Then it returns None (span too large).
        assert!(range.is_none());
    }

    #[rstest::rstest]
    fn compute_affected_range_returns_none_for_none_inputs() {
        // Given no changed lines.
        let range = compute_affected_line_range(None, None, 10);

        // Then it returns None.
        assert!(range.is_none());
    }

    #[rstest::rstest]
    fn build_anchor_block_produces_block() {
        // Given a 5-line result with changes on lines 2-3.
        let content = "alpha\nBETA\nGAMMA\ndelta\nepsilon\n";

        // When building the anchor block.
        let block = build_anchor_block(content, Some(2), Some(3));

        // Then it produces a Block variant.
        match &block {
            AnchorBlock::Block { text, start, end } => {
                assert!(text.starts_with("--- Anchors"));
                assert_eq!(*start, 1); // 2 - 2 = 1
                assert_eq!(*end, 5); // 3 + 2 = 5
                assert!(text.contains("|BETA"));
                assert!(text.contains("|GAMMA"));
            }
            _ => panic!("expected Block, got {block:?}"),
        }
    }

    #[rstest::rstest]
    fn build_anchor_block_omits_for_large_span() {
        // Given a 20-line result with changes spanning 15 lines.
        #[allow(clippy::format_collect, reason = "format in map is intentional")]
        let content: String = (1..=20).map(|i| format!("line {i}\n")).collect();

        // When building the anchor block.
        let block = build_anchor_block(&content, Some(1), Some(15));

        // Then it omits anchors.
        assert!(matches!(block, AnchorBlock::Omitted));
    }

    #[rstest::rstest]
    fn format_success_includes_anchor_block() {
        // Given an anchor block.
        let content = "alpha\nBETA\ngamma\n";
        let block = build_anchor_block(content, Some(2), Some(2));

        // When formatting success response.
        let text = format_success_response("test.txt", &block, &[]);

        // Then it includes the anchor block.
        assert!(text.contains("--- Anchors"));
    }

    #[rstest::rstest]
    fn noop_response_message() {
        // Given noop edits.
        let noop = super::super::engine::NoopEdit {
            edit_index: 0,
            loc: "2#WS".to_owned(),
            current_content: "beta".to_owned(),
        };

        // When formatting noop response.
        let text = format_noop_response("test.txt", &[noop], &[]);

        // Then it says no changes.
        assert!(text.contains("No changes made to test.txt"));
        assert!(text.contains("identical to current content"));
    }
}
