//! The Markdown export format.
//!
//! Markdown is the paste-friendly format: it goes into a PR description, a
//! bug report, or a chat window without a renderer, so it stays close to the
//! model's own markdown and adds only the structure a transcript needs —
//! a heading per entry, and a labelled block for the kinds that HTML can
//! collapse.
//!
//! There is no disclosure primitive in Markdown, so tool calls, tool
//! results, and reasoning become `<details>`-free labelled sections under a
//! bold summary line, with the body in a fenced block. Fences are lengthened
//! past the longest run of backticks in the body so a body containing
//! ``` cannot terminate the block early.
//!
//! Images are referenced by their media type and byte count rather than
//! inlined: a Markdown file that is meant to be pasted cannot carry
//! megabytes of base64, and the alternative — a path that only exists in the
//! exporting machine's memory — would be a broken link.

use crate::document::ExportDocument;
use crate::document::ExportEntry;
use crate::format::ExportFormat;

/// The visible label above a prose entry.
fn prose_label(class: &str) -> &'static str {
    match class {
        "user" => "You",
        "assistant" => "Assistant",
        "actor" => "Actor",
        "system" => "System",
        "error" => "Error",
        "compaction" => "Compaction summary",
        "annotation" => "Sources",
        _ => "Entry",
    }
}

/// A fence long enough to survive the longest backtick run in `body`.
///
/// Three backticks is the minimum; a body containing ``` needs four, and so
/// on. Without this, a tool result quoting markdown would break out of its
/// block and reflow the rest of the document.
#[must_use]
pub fn fence_for(body: &str) -> String {
    let mut longest = 0_usize;
    let mut run = 0_usize;
    for ch in body.chars() {
        if ch == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    "`".repeat(longest.max(2) + 1)
}

/// Renders one entry as a labelled section.
fn render_entry(entry: &ExportEntry, out: &mut String) {
    use std::fmt::Write as _;
    out.push_str("### ");
    out.push_str(prose_label(entry.class));
    out.push('\n');
    if let Some(summary) = &entry.summary {
        let _ = writeln!(out, "**{summary}**\n");
    }
    if entry.is_disclosure {
        let fence = fence_for(&entry.body);
        out.push_str(&fence);
        out.push('\n');
        out.push_str(&entry.body);
        if !entry.body.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&fence);
        out.push_str("\n\n");
    } else {
        out.push_str(&entry.body);
        out.push_str("\n\n");
    }
    for image in &entry.images {
        let _ = writeln!(out, "_(attached image: {} bytes)_\n", image.data.len());
    }
}

/// Renders a whole document as a Markdown file.
#[must_use]
pub fn render_markdown_document(doc: &ExportDocument) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(out, "# {}\n", doc.title);
    let _ = writeln!(out, "- **Model:** {}", doc.model);
    let _ = writeln!(out, "- **Working directory:** {}", doc.cwd.display());
    let _ = writeln!(out, "- **Created:** {}", doc.created_at);
    let _ = writeln!(out, "- **Updated:** {}", doc.updated_at);
    let _ = writeln!(out, "- **Entries:** {}\n", doc.entries.len());
    for entry in &doc.entries {
        render_entry(entry, &mut out);
    }
    out
}

/// The Markdown [`ExportFormat`].
///
/// Named only here and in `format.rs`'s resolver arm; no other call site
/// knows this type exists.
#[derive(Debug, Clone, Copy)]
pub struct MarkdownExport;

impl ExportFormat for MarkdownExport {
    fn extension(&self) -> &'static str {
        "md"
    }

    fn render(&self, doc: &ExportDocument) -> String {
        render_markdown_document(doc)
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use super::*;
    use jiff::Timestamp;

    fn doc_with(entries: Vec<ExportEntry>) -> ExportDocument {
        ExportDocument {
            title: "Test session".to_owned(),
            model: "test-model".to_owned(),
            cwd: std::path::PathBuf::from("/tmp/work"),
            created_at: Timestamp::UNIX_EPOCH,
            updated_at: Timestamp::UNIX_EPOCH,
            session_id: "s-1".to_owned(),
            entries,
        }
    }

    fn entry_of(class: &'static str, body: &str) -> ExportEntry {
        ExportEntry {
            class,
            body: body.to_owned(),
            summary: None,
            is_disclosure: false,
            images: Vec::new(),
            tool_status: None,
            line_count: None,
            token_count: None,
            timestamp: Timestamp::UNIX_EPOCH,
        }
    }

    #[rstest::rstest]
    fn markdown_source_passes_through_as_markdown() {
        // Given an assistant message that is already markdown.
        let doc = doc_with(vec![entry_of("assistant", "## Findings\n\n- one\n- two")]);

        // When rendering it.
        let md = render_markdown_document(&doc);

        // Then the model's own markdown survives, unescaped and unrendered.
        assert!(md.contains("## Findings"));
        assert!(md.contains("- one"));
    }

    #[rstest::rstest]
    fn disclosure_entry_renders_as_a_fenced_block() {
        // Given a tool result that collapses.
        let mut entry = entry_of("tool_result", "line one\nline two");
        entry.is_disclosure = true;
        entry.summary = Some("read · success · 2 lines".to_owned());
        let doc = doc_with(vec![entry]);

        // When rendering it.
        let md = render_markdown_document(&doc);

        // Then it is a labelled section with a fenced body.
        assert!(md.contains("**read · success · 2 lines**"));
        assert!(md.contains("```"));
        assert!(md.contains("line one\nline two"));
    }

    #[rstest::rstest]
    fn fence_grows_past_a_backtick_run_in_the_body() {
        // Given a body containing a triple-backtick run.
        let body = "before\n```\nnot a fence\n```\nafter";

        // When picking a fence for it.
        let fence = fence_for(body);

        // Then the fence is longer than the run inside the body.
        assert!(fence.len() > 3);
    }

    #[rstest::rstest]
    fn plain_body_uses_a_three_backtick_fence() {
        // Given a body with no backticks.
        let fence = fence_for("nothing special");

        // Then the ordinary three-backtick fence is used.
        assert_eq!(fence, "```");
    }

    #[rstest::rstest]
    fn document_header_carries_the_session_metadata() {
        // Given a one-entry document.
        let doc = doc_with(vec![entry_of("user", "hi")]);

        // When rendering it.
        let md = render_markdown_document(&doc);

        // Then the metadata block is present.
        assert!(md.starts_with("# Test session"));
        assert!(md.contains("**Model:** test-model"));
        assert!(md.contains("/tmp/work"));
        assert!(md.contains("**Entries:** 1"));
    }
}
