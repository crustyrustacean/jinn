//! Hashline edit engine — validation, resolution, and bottom-up application.
//!
//! This module implements the core edit pipeline:
//! 1. Parse raw JSON edit requests into typed operations
//! 2. Validate anchors against current file content
//! 3. Resolve edits to byte spans
//! 4. Detect conflicts (overlapping spans, same-boundary inserts)
//! 5. Apply spans bottom-up (highest byte offset first)
//! 6. Track first/last changed lines for response formatting

use super::hash::{
    Anchor, HashMismatch, assert_no_display_prefixes, compute_line_hash, format_tag, parse_anchor,
};
use std::fmt::Write;

use unicode_segmentation::UnicodeSegmentation;

// ─── Constants ──────────────────────────────────────────────────────────

/// Context lines around mismatches in error messages.
const MISMATCH_CONTEXT: usize = 2;

// ─── Types ──────────────────────────────────────────────────────────────

/// The edit operation type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditOp {
    Replace,
    Append,
    Prepend,
    ReplaceText,
}

impl EditOp {
    /// Parses an op string, returning `None` for unknown ops.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "replace" => Some(Self::Replace),
            "append" => Some(Self::Append),
            "prepend" => Some(Self::Prepend),
            "replace_text" => Some(Self::ReplaceText),
            _ => None,
        }
    }
}

/// A resolved edit operation with parsed anchors.
#[derive(Debug, Clone)]
pub enum HashlineEdit {
    /// Replace the line at `pos`, or the inclusive range `pos`..`end`, with `lines`.
    Replace {
        pos: Anchor,
        end: Option<Anchor>,
        lines: Vec<String>,
    },
    /// Insert `lines` after `pos`; no `pos` means append at EOF.
    Append {
        pos: Option<Anchor>,
        lines: Vec<String>,
    },
    /// Insert `lines` before `pos`; no `pos` means insert at BOF.
    Prepend {
        pos: Option<Anchor>,
        lines: Vec<String>,
    },
    /// Replace the one exact unique occurrence of `old_text` with `new_text`.
    ReplaceText { old_text: String, new_text: String },
}

/// Schema-level edit as received from the tool layer.
#[derive(Debug, Clone)]
pub struct RawEdit {
    pub op: String,
    pub pos: Option<String>,
    pub end: Option<String>,
    pub lines: Option<LinesInput>,
    pub old_text: Option<String>,
    pub new_text: Option<String>,
}

/// The `lines` field can be an array of strings or null.
#[derive(Debug, Clone)]
pub enum LinesInput {
    Array(Vec<String>),
    Null,
}

impl LinesInput {
    /// Converts to a `Vec<String>`, treating `Null` as an empty vec.
    pub fn into_lines(self) -> Vec<String> {
        match self {
            Self::Array(v) => v,
            Self::Null => Vec::new(),
        }
    }
}

/// A NOOP edit (replacement identical to current content).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoopEdit {
    /// Index of the edit in the request.
    pub edit_index: usize,
    /// Location description (e.g., "5#WS").
    pub loc: String,
    /// The current content at that location.
    pub current_content: String,
}

/// The result of applying hashline edits.
#[derive(Debug, Clone)]
pub struct EditResult {
    /// The resulting file content (LF-normalized).
    pub content: String,
    /// 1-indexed first changed line, if any.
    pub first_changed_line: Option<usize>,
    /// 1-indexed last changed line, if any.
    pub last_changed_line: Option<usize>,
    /// Warnings (e.g., boundary duplication).
    pub warnings: Vec<String>,
    /// Edits that produced no change.
    pub noop_edits: Vec<NoopEdit>,
}

/// A resolved line-range span for applying an edit.
#[derive(Debug, Clone)]
struct ResolvedSpan {
    /// \"replace\", \"append\", or \"prepend\".
    kind: &'static str,
    /// Edit index in the original request.
    index: usize,
    /// Human-readable label for error messages.
    label: String,
    /// Start line (1-indexed, inclusive).
    start_line: usize,
    /// End line (1-indexed, inclusive). For inserts, equals start_line.
    end_line: usize,
    /// The replacement lines (one String per line, no embedded newlines).
    replacement_lines: Vec<String>,
}

// ─── Parsing ────────────────────────────────────────────────────────────

/// Resolves raw tool-schema edits into typed internal representations.
///
/// # Errors
///
/// Returns a descriptive error string if any edit is malformed.
pub fn resolve_edit_anchors(edits: &[RawEdit]) -> Result<Vec<HashlineEdit>, String> {
    let mut result = Vec::new();
    for edit in edits {
        let op = EditOp::from_str(&edit.op).ok_or_else(|| {
            format!(
                "[E_BAD_OP] Unknown edit op \"{}\". Expected \"replace\", \"append\", \"prepend\", or \"replace_text\".",
                edit.op
            )
        })?;

        match op {
            EditOp::Replace => {
                let pos_str = edit
                    .pos
                    .as_deref()
                    .ok_or_else(|| "[E_BAD_OP] Replace requires a \"pos\" anchor.".to_owned())?;
                let pos = parse_anchor(pos_str)?;
                let end = edit.end.as_deref().map(parse_anchor).transpose()?;
                let lines = edit
                    .lines
                    .clone()
                    .map(LinesInput::into_lines)
                    .unwrap_or_default();
                assert_no_display_prefixes(&lines)?;
                result.push(HashlineEdit::Replace { pos, end, lines });
            }
            EditOp::Append => {
                if edit.end.is_some() {
                    return Err(
                        "[E_BAD_OP] Append does not support \"end\". Use \"pos\" or omit it for EOF."
                            .to_owned(),
                    );
                }
                let pos = edit.pos.as_deref().map(parse_anchor).transpose()?;
                let lines = edit
                    .lines
                    .clone()
                    .map(LinesInput::into_lines)
                    .unwrap_or_default();
                assert_no_display_prefixes(&lines)?;
                result.push(HashlineEdit::Append { pos, lines });
            }
            EditOp::Prepend => {
                if edit.end.is_some() {
                    return Err(
                        "[E_BAD_OP] Prepend does not support \"end\". Use \"pos\" or omit it for BOF."
                            .to_owned(),
                    );
                }
                let pos = edit.pos.as_deref().map(parse_anchor).transpose()?;
                let lines = edit
                    .lines
                    .clone()
                    .map(LinesInput::into_lines)
                    .unwrap_or_default();
                assert_no_display_prefixes(&lines)?;
                result.push(HashlineEdit::Prepend { pos, lines });
            }
            EditOp::ReplaceText => {
                let old_text = edit.old_text.as_deref().unwrap_or("").to_owned();
                let new_text = edit.new_text.as_deref().unwrap_or("").to_owned();
                if old_text.is_empty() {
                    return Err("[E_BAD_OP] replace_text requires non-empty oldText.".to_owned());
                }
                result.push(HashlineEdit::ReplaceText { old_text, new_text });
            }
        }
    }
    Ok(result)
}

// ─── Validation ─────────────────────────────────────────────────────────

/// Validates all anchors in the edits against the current file content.
///
/// Returns a list of mismatches. If empty, all anchors are valid.
pub fn validate_anchors(edits: &[HashlineEdit], file_lines: &[&str]) -> Vec<HashMismatch> {
    let mut mismatches = Vec::new();
    let mut seen_mismatch_lines = std::collections::HashSet::new();

    for edit in edits {
        match edit {
            HashlineEdit::Replace { pos, end, .. } => {
                validate_anchor(pos, file_lines, &mut mismatches, &mut seen_mismatch_lines);
                if let Some(end_anchor) = end {
                    if pos.line > end_anchor.line {
                        mismatches.push(HashMismatch {
                            line: pos.line,
                            expected: pos.hash.clone(),
                            actual: format!("(range start {} > end {})", pos.line, end_anchor.line),
                        });
                        continue;
                    }
                    validate_anchor(
                        end_anchor,
                        file_lines,
                        &mut mismatches,
                        &mut seen_mismatch_lines,
                    );
                }
            }
            HashlineEdit::Append { pos, .. } | HashlineEdit::Prepend { pos, .. } => {
                if let Some(anchor) = pos {
                    validate_anchor(
                        anchor,
                        file_lines,
                        &mut mismatches,
                        &mut seen_mismatch_lines,
                    );
                }
            }
            HashlineEdit::ReplaceText { .. } => {}
        }
    }

    mismatches
}

fn validate_anchor(
    anchor: &Anchor,
    file_lines: &[&str],
    mismatches: &mut Vec<HashMismatch>,
    seen: &mut std::collections::HashSet<usize>,
) {
    if anchor.line < 1 || anchor.line > file_lines.len() {
        mismatches.push(HashMismatch {
            line: anchor.line,
            expected: anchor.hash.clone(),
            actual: format!(
                "(line {} out of range; file has {} lines)",
                anchor.line,
                file_lines.len()
            ),
        });
        return;
    }
    let Some(&line_content) = file_lines.get(anchor.line - 1) else {
        return;
    };
    let actual = compute_line_hash(anchor.line, line_content);
    if actual != anchor.hash && !seen.contains(&anchor.line) {
        seen.insert(anchor.line);
        mismatches.push(HashMismatch {
            line: anchor.line,
            expected: anchor.hash.clone(),
            actual: actual.to_owned(),
        });
    }
}

/// Formats a mismatch error with context lines and `>>>` markers.
pub fn format_mismatch_error(mismatches: &[HashMismatch], file_lines: &[&str]) -> String {
    let mismatch_lines: std::collections::HashSet<usize> =
        mismatches.iter().map(|m| m.line).collect();

    let mut display_lines = std::collections::HashSet::new();
    for m in mismatches {
        for i in m.line.saturating_sub(MISMATCH_CONTEXT)
            ..=(m.line + MISMATCH_CONTEXT).min(file_lines.len())
        {
            display_lines.insert(i);
        }
    }
    for line in &mismatch_lines {
        display_lines.insert(*line);
    }

    let mut sorted: Vec<usize> = display_lines.into_iter().collect();
    sorted.sort_unstable();

    let max_line = sorted.last().copied().unwrap_or(1);
    let width = format!("{max_line}").len();

    let n = mismatches.len();
    let mut out = String::new();
    let _ = write!(
        out,
        "[E_STALE_ANCHOR] {n} stale anchor{}. Retry with the >>> LINE#HASH lines below; keep both endpoints for range replaces.\n\n",
        if n > 1 { "s" } else { "" }
    );

    let mut prev = 0usize;
    for num in sorted {
        if num == 0 || num > file_lines.len() {
            continue;
        }
        if prev != 0 && num > prev + 1 {
            out.push_str("    ...\n");
        }
        prev = num;

        let Some(&content) = file_lines.get(num - 1) else {
            continue;
        };
        let h = compute_line_hash(num, content);
        if mismatch_lines.contains(&num) {
            let _ = writeln!(out, ">>> {num:>width$}#{h}|{content}");
        } else {
            let _ = writeln!(out, "    {num:>width$}#{h}|{content}");
        }
    }

    out
}

// ─── Application ────────────────────────────────────────────────────────

/// Resolved, deduplicated, conflict-checked, sorted edit spans ready for application.
struct EditPlan {
    spans: Vec<ResolvedSpan>,
    warnings: Vec<String>,
    noop_edits: Vec<NoopEdit>,
}

/// Resolves raw edits into a sorted, deduplicated, conflict-checked plan.
///
/// Returns an [`EditPlan`] containing all spans ready for sequential application.
fn build_edit_plan(
    edits: &[HashlineEdit],
    content: &str,
    file_lines: &[&str],
    line_starts: &[usize],
    has_terminal_newline: bool,
) -> Result<EditPlan, String> {
    let mut warnings = Vec::new();
    let mut noop_edits = Vec::new();
    let mut spans = Vec::new();

    for (index, edit) in edits.iter().enumerate() {
        let span = resolve_edit_to_span(
            edit,
            index,
            content,
            file_lines,
            line_starts,
            has_terminal_newline,
            &mut noop_edits,
        )?;

        let Some(span) = span else {
            continue;
        };

        check_boundary_duplication(edit, file_lines, &mut warnings);
        spans.push(span);
    }

    // Deduplicate identical spans.
    let mut seen_keys = std::collections::HashSet::new();
    spans.retain(|s| {
        let key = format!(
            "{}:{}:{}:{}",
            s.kind,
            s.start_line,
            s.end_line,
            s.replacement_lines.join("\\n")
        );
        seen_keys.insert(key)
    });

    assert_no_conflicting_spans(&spans)?;

    // Sort top-to-bottom: lowest start_line first, then by kind priority, then by index.
    spans.sort_by(|a, b| {
        if a.start_line == b.start_line {
            let priority = |k: &str| match k {
                "prepend" => 0,
                "replace" => 1,
                "append" => 2,
                _ => 3,
            };
            match priority(a.kind).cmp(&priority(b.kind)) {
                std::cmp::Ordering::Equal => a.index.cmp(&b.index),
                other => other,
            }
        } else {
            a.start_line.cmp(&b.start_line)
        }
    });

    Ok(EditPlan {
        spans,
        warnings,
        noop_edits,
    })
}

/// Rebuilds the file by applying sorted spans to the original lines.
///
/// Walks a cursor through the original file, emitting unchanged lines and
/// replacement content at each span's position.
fn rebuild_file_from_spans(
    file_lines: &[&str],
    spans: &[ResolvedSpan],
    has_terminal_newline: bool,
) -> Result<String, String> {
    let mut output_lines: Vec<String> = Vec::new();
    let mut cursor: usize = 1;

    // Push the original file line at `cursor - 1`, returning an error if the
    // cursor escaped the file bounds (malformed span from the LLM).
    macro_rules! push_original_line {
        () => {
            output_lines.push(
                file_lines
                    .get(cursor.checked_sub(1).unwrap_or(0))
                    .ok_or_else(|| {
                        format!(
                            "cursor {cursor} out of bounds for file with {} lines",
                            file_lines.len()
                        )
                    })?
                    .to_string(),
            );
        };
    }
    for span in spans {
        match span.kind {
            "replace" => {
                while cursor < span.start_line {
                    push_original_line!();
                    cursor += 1;
                }
                cursor = span.end_line + 1;
                output_lines.extend(span.replacement_lines.iter().cloned());
            }
            "append" => {
                while cursor <= span.start_line && cursor <= file_lines.len() {
                    push_original_line!();
                    cursor += 1;
                }
                output_lines.extend(span.replacement_lines.iter().cloned());
            }
            "prepend" => {
                while cursor < span.start_line {
                    push_original_line!();
                    cursor += 1;
                }
                output_lines.extend(span.replacement_lines.iter().cloned());
            }
            _ => {}
        }
    }

    while cursor <= file_lines.len() {
        push_original_line!();
        cursor += 1;
    }

    // If the original had a terminal newline, the split produced an empty trailing
    // element. Remove it before joining so we don't get a double newline.
    if has_terminal_newline && output_lines.last().is_some_and(String::is_empty) {
        output_lines.pop();
    }

    let mut result = output_lines.join("\n");
    if has_terminal_newline {
        result.push('\n');
    }

    Ok(result)
}

/// Applies validated hashline edits to the file content.
///
/// Pipeline: prepare file index → build edit plan → rebuild file → compute changed range.
pub fn apply_hashline_edits(content: &str, edits: &[HashlineEdit]) -> Result<EditResult, String> {
    if edits.is_empty() {
        return Ok(EditResult {
            content: content.to_owned(),
            first_changed_line: None,
            last_changed_line: None,
            warnings: Vec::new(),
            noop_edits: Vec::new(),
        });
    }

    let file_lines: Vec<&str> = content.split('\n').collect();
    let has_terminal_newline = content.ends_with('\n');

    // Build byte-offset index for each line start.
    let line_starts = {
        let mut starts = Vec::with_capacity(file_lines.len());
        let mut offset = 0;
        for (i, line) in file_lines.iter().enumerate() {
            starts.push(offset);
            offset += line.len();
            if i < file_lines.len() - 1 {
                offset += 1; // newline
            }
        }
        starts
    };

    let plan = build_edit_plan(
        edits,
        content,
        &file_lines,
        &line_starts,
        has_terminal_newline,
    )?;

    let result = rebuild_file_from_spans(&file_lines, &plan.spans, has_terminal_newline)?;

    let changed_range = compute_changed_line_range(content, &result);

    Ok(EditResult {
        content: result,
        first_changed_line: changed_range.0,
        last_changed_line: changed_range.1,
        warnings: plan.warnings,
        noop_edits: plan.noop_edits,
    })
}

/// Read-only context shared by every edit-resolver helper.
///
/// Bundles the file geometry + a sink for NOOP records so each resolver
/// only takes its edit-specific parameters on top of `&self`.
struct ResolveCtx<'a> {
    content: &'a str,
    file_lines: &'a [&'a str],
    line_starts: &'a [usize],
    has_terminal_newline: bool,
    noop_edits: &'a mut Vec<NoopEdit>,
}

impl ResolveCtx<'_> {
    /// Records a NOOP edit and returns the "resolved to nothing" sentinel.
    fn record_noop(
        &mut self,
        index: usize,
        loc: String,
        current_content: String,
    ) -> Option<ResolvedSpan> {
        self.noop_edits.push(NoopEdit {
            edit_index: index,
            loc,
            current_content,
        });
        None
    }
}

/// Resolves a single edit to a byte span (or `None` for NOOPs).
fn resolve_edit_to_span(
    edit: &HashlineEdit,
    index: usize,
    content: &str,
    file_lines: &[&str],
    line_starts: &[usize],
    has_terminal_newline: bool,
    noop_edits: &mut Vec<NoopEdit>,
) -> Result<Option<ResolvedSpan>, String> {
    let mut ctx = ResolveCtx {
        content,
        file_lines,
        line_starts,
        has_terminal_newline,
        noop_edits,
    };
    let label = describe_edit(edit);
    match edit {
        HashlineEdit::Replace { pos, end, lines } => Ok(resolve_replace_span(
            &mut ctx,
            pos,
            end.as_ref(),
            lines,
            index,
            &label,
        )),
        HashlineEdit::Append { pos, lines } => Ok(resolve_append_span(
            &mut ctx,
            pos.as_ref(),
            lines,
            index,
            &label,
        )),
        HashlineEdit::Prepend { pos, lines } => Ok(resolve_prepend_span(
            &mut ctx,
            pos.as_ref(),
            lines,
            index,
            &label,
        )),
        HashlineEdit::ReplaceText { old_text, new_text } => {
            resolve_replace_text_span(&mut ctx, old_text, new_text, index, &label)
        }
    }
}

/// Resolves a `Replace` edit (line-range swap) to a byte span.
fn resolve_replace_span(
    ctx: &mut ResolveCtx<'_>,
    pos: &Anchor,
    end: Option<&Anchor>,
    lines: &[String],
    index: usize,
    label: &str,
) -> Option<ResolvedSpan> {
    let start_line = pos.line;
    let end_line = end.map_or(start_line, |a| a.line);

    // NOOP check
    let original = ctx
        .file_lines
        .get(start_line - 1..end_line.min(ctx.file_lines.len()))?;
    let original: Vec<&str> = original.to_vec();
    if original.len() == lines.len() && original.iter().zip(lines.iter()).all(|(a, b)| a == b) {
        return ctx.record_noop(
            index,
            format!("{}#{}", pos.line, pos.hash),
            original.join("\n"),
        );
    }

    let _start_byte = ctx.line_starts.get(start_line - 1)?;
    let end_byte_start = ctx.line_starts.get(end_line - 1).copied();
    let end_line_content = ctx.file_lines.get(end_line - 1);
    let _end_byte = end_byte_start? + end_line_content?.len();
    let _replacement = lines.join("\n");
    Some(ResolvedSpan {
        kind: "replace",
        index,
        label: label.to_owned(),
        start_line,
        end_line,
        replacement_lines: lines.to_vec(),
    })
}

/// Resolves an `Append` edit (insert after `pos`, or at EOF) to a line-range span.
fn resolve_append_span(
    ctx: &mut ResolveCtx<'_>,
    pos: Option<&Anchor>,
    lines: &[String],
    index: usize,
    label: &str,
) -> Option<ResolvedSpan> {
    if lines.is_empty() {
        // NOOP: inserting nothing
        return ctx.record_noop(
            index,
            pos.map_or("EOF".to_owned(), |a| format!("{}#{}", a.line, a.hash)),
            String::new(),
        );
    }

    let start_line = if ctx.content.is_empty() {
        // Empty file — insert at line 1
        1
    } else if let Some(anchor) = pos {
        // Insert after anchor.line
        // If anchor points to the sentinel line (empty trailing element from
        // terminal newline), treat it as "after last real line" — emit all real lines
        // then insert.
        if ctx.has_terminal_newline && anchor.line == ctx.file_lines.len() {
            ctx.file_lines.len() - 1 // last real line
        } else {
            anchor.line
        }
    } else {
        // Append at EOF — insert after last real line
        if ctx.has_terminal_newline && !ctx.file_lines.is_empty() {
            ctx.file_lines.len() - 1
        } else {
            ctx.file_lines.len()
        }
    };
    Some(ResolvedSpan {
        kind: "append",
        index,
        label: label.to_owned(),
        start_line,
        end_line: start_line,
        replacement_lines: lines.to_vec(),
    })
}

/// Resolves a `Prepend` edit (insert before `pos`, or at BOF) to a line-range span.
fn resolve_prepend_span(
    ctx: &mut ResolveCtx<'_>,
    pos: Option<&Anchor>,
    lines: &[String],
    index: usize,
    label: &str,
) -> Option<ResolvedSpan> {
    if lines.is_empty() {
        return ctx.record_noop(
            index,
            pos.map_or("BOF".to_owned(), |a| format!("{}#{}", a.line, a.hash)),
            String::new(),
        );
    }

    let start_line = pos.map_or(1, |a| a.line);

    Some(ResolvedSpan {
        kind: "prepend",
        index,
        label: label.to_owned(),
        start_line,
        end_line: start_line,
        replacement_lines: lines.to_vec(),
    })
}

/// Resolves a `ReplaceText` edit (unique-text swap) to a line-range span.
///
/// Unlike the other resolvers this can fail when `old_text` is absent or
/// not unique, so it keeps the `Result` return.
#[expect(
    clippy::expect_used,
    reason = "line offsets from line_number_at_byte are valid indices"
)]
fn resolve_replace_text_span(
    ctx: &mut ResolveCtx<'_>,
    old_text: &str,
    new_text: &str,
    index: usize,
    label: &str,
) -> Result<Option<ResolvedSpan>, String> {
    if old_text == new_text {
        return Ok(ctx.record_noop(
            index,
            format!("replace_text \"{old_text}\""),
            old_text.to_owned(),
        ));
    }

    let found = find_exact_unique_match(ctx.content, old_text);
    match found {
        None => {
            // Check if text doesn't exist at all vs. exists multiple times
            if !ctx.content.contains(old_text) {
                return Err(format!(
                    "[E_NOT_FOUND] replace_text: \"{old_text}\" not found in file."
                ));
            }
            Err(format!(
                "[E_NOT_UNIQUE] replace_text: \"{old_text}\" appears more than once."
            ))
        }
        Some((byte_start, byte_end)) => {
            let start_line = line_number_at_byte(ctx.line_starts, byte_start);
            let end_line = line_number_at_byte(ctx.line_starts, byte_end);
            let prefix_len = byte_start
                - ctx
                    .line_starts
                    .get(start_line - 1)
                    .expect("start_line from line_number_at_byte");
            let suffix_start = byte_end
                - ctx
                    .line_starts
                    .get(end_line - 1)
                    .expect("end_line from line_number_at_byte");
            let first_line_prefix = ctx
                .file_lines
                .get(start_line - 1)
                .expect("start_line from line_number_at_byte")
                .get(..prefix_len)
                .expect("prefix_len is byte offset within line");
            let last_line_suffix = ctx
                .file_lines
                .get(end_line - 1)
                .expect("end_line from line_number_at_byte")
                .get(suffix_start..)
                .expect("suffix_start is byte offset within line");
            let full_replacement = format!("{first_line_prefix}{new_text}{last_line_suffix}");
            let replacement_lines: Vec<String> =
                full_replacement.split('\n').map(String::from).collect();
            Ok(Some(ResolvedSpan {
                kind: "replace",
                index,
                label: label.to_owned(),
                start_line,
                end_line,
                replacement_lines,
            }))
        }
    }
}

/// Returns the 1-indexed line number containing the given byte offset.
///
/// `line_starts[i]` is the byte offset where line `i+1` begins.
/// The function finds the last line whose start is <= `byte`.
/// Finds the exact unique match of `old_text` in `content`.
///
/// Returns `(start, end)` byte offsets or an error via `None` (the caller
/// should return a descriptive error).
fn find_exact_unique_match(content: &str, old_text: &str) -> Option<(usize, usize)> {
    if old_text.is_empty() {
        return None;
    }

    let mut matches = Vec::new();
    let mut from = 0;
    while from + old_text.len() <= content.len() {
        let Some(idx) = content.get(from..).and_then(|s| s.find(old_text)) else {
            break;
        };
        let abs = from + idx;
        matches.push(abs);
        from = abs + 1;
    }

    if matches.len() > 1 {
        // Multiple matches — not unique
        return None;
    }

    let start = *matches.first()?;
    Some((start, start + old_text.len()))
}

/// Checks for boundary duplication and adds a warning if detected.
#[expect(clippy::expect_used, reason = "infallible")]
fn check_boundary_duplication(
    edit: &HashlineEdit,
    file_lines: &[&str],
    warnings: &mut Vec<String>,
) {
    let (lines, end_line) = match edit {
        HashlineEdit::Replace { pos, end, lines } => {
            let end_line = end.as_ref().map_or(pos.line, |a| a.line);
            (lines, end_line)
        }
        _ => return,
    };

    if lines.is_empty() {
        return;
    }

    let next_idx = end_line; // 0-indexed next line after end
    let Some(next) = file_lines.get(next_idx) else {
        return;
    };
    let last_repl = lines.last().expect("lines non-empty").trim();
    if !last_repl.is_empty()
        && last_repl.chars().any(char::is_alphanumeric)
        && last_repl == next.trim()
    {
        let next_tag = format_tag(end_line + 1, next);
        warnings.push(format!(
            "Potential boundary duplication after {}: the replacement ends with a line that matches the next surviving line after trim.",
            describe_edit(edit)
        ));
        let _ = next_tag; // suppress unused warning
    }
}

/// Describes an edit for error messages.
fn describe_edit(edit: &HashlineEdit) -> String {
    match edit {
        HashlineEdit::Replace { pos, end, .. } => {
            if let Some(end_anchor) = end {
                format!(
                    "replace {}#{}-{}#{}",
                    pos.line, pos.hash, end_anchor.line, end_anchor.hash
                )
            } else {
                format!("replace {}#{}", pos.line, pos.hash)
            }
        }
        HashlineEdit::Append { pos, .. } => pos.as_ref().map_or("append at EOF".to_owned(), |a| {
            format!("append after {}#{}", a.line, a.hash)
        }),
        HashlineEdit::Prepend { pos, .. } => {
            pos.as_ref().map_or("prepend at BOF".to_owned(), |a| {
                format!("prepend before {}#{}", a.line, a.hash)
            })
        }
        HashlineEdit::ReplaceText { old_text, .. } => {
            let preview = if old_text.len() > 32 {
                let truncated: String = old_text.graphemes(true).take(29).collect();
                format!("{truncated}...")
            } else {
                old_text.replace('\n', "\\n")
            };
            format!("replace_text \"{preview}\"")
        }
    }
}

/// Rejects overlapping spans and same-boundary inserts.
fn assert_no_conflicting_spans(spans: &[ResolvedSpan]) -> Result<(), String> {
    for (i, left) in spans.iter().enumerate() {
        for right in spans.iter().skip(i + 1) {
            // Two inserts at same boundary
            let left_is_insert = left.kind == "append" || left.kind == "prepend";
            let right_is_insert = right.kind == "append" || right.kind == "prepend";
            if left_is_insert && right_is_insert {
                if left.start_line == right.start_line && left.kind == right.kind {
                    return Err(format!(
                        "[E_EDIT_CONFLICT] Conflicting edits in a single request: edit {} ({}) and edit {} ({}) target the same insertion boundary.",
                        left.index, left.label, right.index, right.label
                    ));
                }
                continue;
            }

            // Two replaces that overlap
            if left.kind == "replace" && right.kind == "replace" {
                if left.start_line <= right.end_line && right.start_line <= left.end_line {
                    return Err(format!(
                        "[E_EDIT_CONFLICT] Conflicting edits in a single request: edit {} ({}) and edit {} ({}) overlap on the same original line range.",
                        left.index, left.label, right.index, right.label
                    ));
                }
                continue;
            }

            // Insert inside a replace
            let (replace_span, insert_span) = if left.kind == "replace" {
                (left, right)
            } else {
                (right, left)
            };

            // For prepend: start_line is the line it inserts before. If that line is
            // within [replace.start_line, replace.end_line], it's a conflict.
            // For append: start_line is the line it inserts after. If that line is
            // within [replace.start_line, replace.end_line) (exclusive end), it's a conflict.
            // Append after the replace's end_line is allowed — it inserts after the range.
            let inside = if insert_span.kind == "append" {
                insert_span.start_line >= replace_span.start_line
                    && insert_span.start_line < replace_span.end_line
            } else {
                insert_span.start_line >= replace_span.start_line
                    && insert_span.start_line <= replace_span.end_line
            };
            if inside {
                return Err(format!(
                    "[E_EDIT_CONFLICT] Conflicting edits in a single request: edit {} ({}) and edit {} ({}) cannot be applied together because one inserts inside a replaced original range.",
                    left.index, left.label, right.index, right.label
                ));
            }
        }
    }
    Ok(())
}

/// Computes the first and last changed line numbers between original and result.
#[expect(
    clippy::items_after_statements,
    reason = "helper placement after main logic"
)]
fn compute_changed_line_range(original: &str, result: &str) -> (Option<usize>, Option<usize>) {
    if original == result {
        return (None, None);
    }

    fn count_visible_lines(text: &str) -> usize {
        if text.is_empty() {
            return 0;
        }
        let lines: Vec<&str> = text.split('\n').collect();
        if text.ends_with('\n') {
            lines.len() - 1
        } else {
            lines.len()
        }
    }

    if original.is_empty() {
        return (Some(1), Some(count_visible_lines(result)));
    }

    // Find first differing byte
    let min_len = original.len().min(result.len());
    let mut first_diff = 0;
    while first_diff < min_len
        && original.as_bytes().get(first_diff) == result.as_bytes().get(first_diff)
    {
        first_diff += 1;
    }

    // Find last differing byte
    let mut last_orig = original.len() as isize - 1;
    let mut last_res = result.len() as isize - 1;
    while last_orig >= first_diff as isize
        && last_res >= first_diff as isize
        && original.as_bytes().get(last_orig as usize) == result.as_bytes().get(last_res as usize)
    {
        last_orig -= 1;
        last_res -= 1;
    }

    fn index_to_line(char_idx: usize, text: &str) -> usize {
        let mut line = 1;
        for (i, b) in text.bytes().enumerate() {
            if i >= char_idx {
                break;
            }
            if b == b'\n' {
                line += 1;
            }
        }
        line
    }

    let first_line = index_to_line(first_diff + 1, result);
    let last_line = if last_res < first_diff as isize {
        if result.is_empty() {
            1
        } else {
            count_visible_lines(result)
        }
    } else {
        index_to_line((last_res + 1) as usize, result)
    };

    (Some(first_line), Some(last_line))
}

fn line_number_at_byte(line_starts: &[usize], byte: usize) -> usize {
    match line_starts.binary_search(&byte) {
        Ok(idx) => idx + 1,
        Err(idx) => idx,
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

    fn anchor(line: usize, hash: impl AsRef<str>) -> Anchor {
        Anchor {
            line,
            hash: hash.as_ref().to_owned(),
        }
    }

    // ─── resolve_edit_anchors tests ──────────────────────────────────

    #[rstest::rstest]
    fn resolve_replace_with_pos_and_end() {
        // Given a raw replace edit with pos and end.
        let edits = vec![RawEdit {
            op: "replace".to_owned(),
            pos: Some("5#WS".to_owned()),
            end: Some("7#TX".to_owned()),
            lines: Some(LinesInput::Array(vec!["a".to_owned(), "b".to_owned()])),
            old_text: None,
            new_text: None,
        }];

        // When resolving.
        let resolved = resolve_edit_anchors(&edits).expect("should resolve");

        // Then we get a Replace variant.
        assert_eq!(resolved.len(), 1);
        assert!(matches!(resolved[0], HashlineEdit::Replace { .. }));
    }

    #[rstest::rstest]
    fn resolve_unknown_op_rejected() {
        // Given an edit with an unknown op.
        let edits = vec![RawEdit {
            op: "bad".to_owned(),
            pos: None,
            end: None,
            lines: None,
            old_text: None,
            new_text: None,
        }];

        // When resolving.
        let result = resolve_edit_anchors(&edits);

        // Then it fails.
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unknown edit op"));
    }

    // ─── apply_hashline_edits tests ──────────────────────────────────

    #[rstest::rstest]
    fn replace_single_line() {
        // Given content and an edit replacing line 2.
        let content = "alpha\nbeta\ngamma\n";
        let pos = anchor(2, compute_line_hash(2, "beta"));
        let edits = vec![HashlineEdit::Replace {
            pos,
            end: None,
            lines: vec!["BETA".to_owned()],
        }];

        // When applying.
        let result = apply_hashline_edits(content, &edits).expect("should apply");

        // Then line 2 is replaced.
        assert_eq!(result.content, "alpha\nBETA\ngamma\n");
        assert_eq!(result.first_changed_line, Some(2));
    }

    #[rstest::rstest]
    fn replace_range() {
        // Given content and an edit replacing lines 2-4.
        let content = "a\nb\nc\nd\ne\n";
        let pos = anchor(2, compute_line_hash(2, "b"));
        let end = anchor(4, compute_line_hash(4, "d"));
        let edits = vec![HashlineEdit::Replace {
            pos,
            end: Some(end),
            lines: vec!["middle".to_owned()],
        }];

        // When applying.
        let result = apply_hashline_edits(content, &edits).expect("should apply");

        // Then lines 2-4 are replaced with one line.
        assert_eq!(result.content, "a\nmiddle\ne\n");
    }

    #[rstest::rstest]
    fn append_at_line() {
        // Given content and an edit appending after line 1.
        let content = "a\nb\nc\n";
        let pos = anchor(1, compute_line_hash(1, "a"));
        let edits = vec![HashlineEdit::Append {
            pos: Some(pos),
            lines: vec!["inserted".to_owned()],
        }];

        // When applying.
        let result = apply_hashline_edits(content, &edits).expect("should apply");

        // Then the line is inserted after line 1.
        assert_eq!(result.content, "a\ninserted\nb\nc\n");
    }

    #[rstest::rstest]
    fn prepend_at_line() {
        // Given content and an edit prepending before line 2.
        let content = "a\nb\nc\n";
        let pos = anchor(2, compute_line_hash(2, "b"));
        let edits = vec![HashlineEdit::Prepend {
            pos: Some(pos),
            lines: vec!["before-b".to_owned()],
        }];

        // When applying.
        let result = apply_hashline_edits(content, &edits).expect("should apply");

        // Then the line is inserted before line 2.
        assert_eq!(result.content, "a\nbefore-b\nb\nc\n");
    }

    #[rstest::rstest]
    fn append_at_eof() {
        // Given content and an edit appending at EOF.
        let content = "a\nb\n";
        let edits = vec![HashlineEdit::Append {
            pos: None,
            lines: vec!["eof-line".to_owned()],
        }];

        // When applying.
        let result = apply_hashline_edits(content, &edits).expect("should apply");

        // Then the line is appended at the end.
        assert!(result.content.ends_with("\neof-line\n"));
    }

    #[rstest::rstest]
    fn prepend_at_bof() {
        // Given content and an edit prepending at BOF.
        let content = "a\nb\n";
        let edits = vec![HashlineEdit::Prepend {
            pos: None,
            lines: vec!["bof-line".to_owned()],
        }];

        // When applying.
        let result = apply_hashline_edits(content, &edits).expect("should apply");

        // Then the line is prepended at the start.
        assert!(result.content.starts_with("bof-line\n"));
    }

    #[rstest::rstest]
    fn replace_text_unique() {
        // Given content with a unique substring.
        let content = "hello world";
        let edits = vec![HashlineEdit::ReplaceText {
            old_text: "world".to_owned(),
            new_text: "rust".to_owned(),
        }];

        // When applying.
        let result = apply_hashline_edits(content, &edits).expect("should apply");

        // Then the text is replaced.
        assert_eq!(result.content, "hello rust");
    }
    #[rstest::rstest]
    fn replace_text_spanning_lines() {
        // Given content where old_text crosses a line boundary.
        let content = "alpha\nbeta\ngamma\n";
        let edits = vec![HashlineEdit::ReplaceText {
            old_text: "beta\ngamma".to_owned(),
            new_text: "BETA\nGAMMA".to_owned(),
        }];

        // When applying.
        let result = apply_hashline_edits(content, &edits).expect("should apply");

        // Then the multi-line replacement is applied.
        assert_eq!(result.content, "alpha\nBETA\nGAMMA\n");
    }

    #[rstest::rstest]
    fn replace_text_mid_line() {
        // Given content where old_text is in the middle of a line.
        let content = "the quick brown fox\njumps\n";
        let edits = vec![HashlineEdit::ReplaceText {
            old_text: "brown".to_owned(),
            new_text: "red".to_owned(),
        }];

        // When applying.
        let result = apply_hashline_edits(content, &edits).expect("should apply");

        // Then only the matched portion is replaced, preserving line structure.
        assert_eq!(result.content, "the quick red fox\njumps\n");
    }

    #[rstest::rstest]
    fn bottom_up_ordering() {
        // Given two replace edits on lines 1 and 5.
        let content = "a\nb\nc\nd\ne\n";
        let tag1 = anchor(1, compute_line_hash(1, "a"));
        let tag5 = anchor(5, compute_line_hash(5, "e"));
        let edits = vec![
            HashlineEdit::Replace {
                pos: tag1,
                end: None,
                lines: vec!["A".to_owned()],
            },
            HashlineEdit::Replace {
                pos: tag5,
                end: None,
                lines: vec!["E".to_owned()],
            },
        ];

        // When applying.
        let result = apply_hashline_edits(content, &edits).expect("should apply");

        // Then both edits are applied correctly.
        assert_eq!(result.content, "A\nb\nc\nd\nE\n");
    }

    #[rstest::rstest]
    fn noop_detection() {
        // Given a replacement that matches current content.
        let content = "alpha\nbeta\n";
        let pos = anchor(2, compute_line_hash(2, "beta"));
        let edits = vec![HashlineEdit::Replace {
            pos,
            end: None,
            lines: vec!["beta".to_owned()],
        }];

        // When applying.
        let result = apply_hashline_edits(content, &edits).expect("should apply");

        // Then the content is unchanged and we get a NOOP.
        assert_eq!(result.content, content);
        assert_eq!(result.noop_edits.len(), 1);
        assert_eq!(result.first_changed_line, None);
    }

    #[rstest::rstest]
    fn overlapping_edits_rejected() {
        // Given two replace edits that overlap.
        let content = "a\nb\nc\n";
        let tag1 = anchor(1, compute_line_hash(1, "a"));
        let tag2 = anchor(2, compute_line_hash(2, "b"));
        let edits = vec![
            HashlineEdit::Replace {
                pos: tag1,
                end: Some(tag2.clone()),
                lines: vec!["X".to_owned()],
            },
            HashlineEdit::Replace {
                pos: tag2,
                end: None,
                lines: vec!["Y".to_owned()],
            },
        ];

        // When applying.
        let result = apply_hashline_edits(content, &edits);

        // Then it fails with a conflict error.
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("E_EDIT_CONFLICT"));
    }

    // ─── crash regression & multi-span interaction tests ──────────

    #[rstest::rstest]
    fn replace_and_append_at_same_boundary() {
        // Given a replace of lines 1-2 and an append after line 2.
        // This is the exact crash scenario: both share the same end boundary,
        // and the old bottom-up algorithm would panic with stale byte offsets.
        let content = "alpha\nbeta\ngamma\n";
        let pos1 = anchor(1, compute_line_hash(1, "alpha"));
        let end1 = anchor(2, compute_line_hash(2, "beta"));
        let pos2 = anchor(2, compute_line_hash(2, "beta"));
        let edits = vec![
            HashlineEdit::Replace {
                pos: pos1,
                end: Some(end1),
                lines: vec!["ALPHA".to_owned(), "BETA".to_owned()],
            },
            HashlineEdit::Append {
                pos: Some(pos2),
                lines: vec!["inserted".to_owned()],
            },
        ];

        // When applying.
        let result = apply_hashline_edits(content, &edits).expect("should apply");

        // Then the replace applies and the appended line follows.
        assert_eq!(result.content, "ALPHA\nBETA\ninserted\ngamma\n");
    }

    #[rstest::rstest]
    fn multiple_non_overlapping_replaces() {
        // Given two non-overlapping replaces at lines 1-2 and 4-5.
        let content = "a\nb\nc\nd\ne\nf\n";
        let pos1 = anchor(1, compute_line_hash(1, "a"));
        let end1 = anchor(2, compute_line_hash(2, "b"));
        let pos2 = anchor(4, compute_line_hash(4, "d"));
        let end2 = anchor(5, compute_line_hash(5, "e"));
        let edits = vec![
            HashlineEdit::Replace {
                pos: pos1,
                end: Some(end1),
                lines: vec!["A".to_owned()],
            },
            HashlineEdit::Replace {
                pos: pos2,
                end: Some(end2),
                lines: vec!["D".to_owned()],
            },
        ];

        // When applying.
        let result = apply_hashline_edits(content, &edits).expect("should apply");

        // Then both ranges are replaced.
        assert_eq!(result.content, "A\nc\nD\nf\n");
    }

    #[rstest::rstest]
    fn replace_and_append_at_eof() {
        // Given a replace mid-file and an append at EOF.
        let content = "a\nb\nc\n";
        let pos1 = anchor(1, compute_line_hash(1, "a"));
        let pos2 = anchor(3, compute_line_hash(3, "c"));
        let edits = vec![
            HashlineEdit::Replace {
                pos: pos1,
                end: None,
                lines: vec!["A".to_owned()],
            },
            HashlineEdit::Append {
                pos: Some(pos2),
                lines: vec!["after-c".to_owned()],
            },
        ];

        // When applying.
        let result = apply_hashline_edits(content, &edits).expect("should apply");

        // Then the replace and append both apply.
        assert_eq!(result.content, "A\nb\nc\nafter-c\n");
    }

    #[rstest::rstest]
    fn prepend_and_replace_same_batch() {
        // Given a prepend before line 1 and a replace at line 3.
        let content = "a\nb\nc\nd\n";
        let pos1 = anchor(1, compute_line_hash(1, "a"));
        let pos2 = anchor(3, compute_line_hash(3, "c"));
        let edits = vec![
            HashlineEdit::Prepend {
                pos: Some(pos1),
                lines: vec!["header".to_owned()],
            },
            HashlineEdit::Replace {
                pos: pos2,
                end: None,
                lines: vec!["C".to_owned()],
            },
        ];

        // When applying.
        let result = apply_hashline_edits(content, &edits).expect("should apply");

        // Then prepend appears before line 1 and replace applies to line 3.
        assert_eq!(result.content, "header\na\nb\nC\nd\n");
    }

    #[rstest::rstest]
    fn replace_and_prepend_at_adjacent_line() {
        // Given a replace of line 1 and a prepend before line 2.
        let content = "alpha\nbeta\ngamma\n";
        let pos1 = anchor(1, compute_line_hash(1, "alpha"));
        let pos2 = anchor(2, compute_line_hash(2, "beta"));
        let edits = vec![
            HashlineEdit::Replace {
                pos: pos1,
                end: None,
                lines: vec!["ALPHA".to_owned()],
            },
            HashlineEdit::Prepend {
                pos: Some(pos2),
                lines: vec!["before-beta".to_owned()],
            },
        ];

        // When applying.
        let result = apply_hashline_edits(content, &edits).expect("should apply");

        // Then replace and prepend both apply correctly.
        assert_eq!(result.content, "ALPHA\nbefore-beta\nbeta\ngamma\n");
    }

    #[rstest::rstest]
    fn single_noop_replace() {
        // Given a replace that produces identical content.
        let content = "a\nb\nc\n";
        let pos = anchor(2, compute_line_hash(2, "b"));
        let edits = vec![HashlineEdit::Replace {
            pos,
            end: None,
            lines: vec!["b".to_owned()],
        }];

        // When applying.
        let result = apply_hashline_edits(content, &edits).expect("should apply");

        // Then the content is unchanged and we get a NOOP.
        assert_eq!(result.content, content);
        assert_eq!(result.noop_edits.len(), 1);
    }

    #[rstest::rstest]
    fn empty_edits_returns_original() {
        // Given no edits.
        let content = "a\nb\n";

        // When applying empty edits.
        let result = apply_hashline_edits(content, &[]).expect("should apply");

        // Then the content is unchanged.
        assert_eq!(result.content, content);
    }

    #[rstest::rstest]
    fn deduplication_of_identical_edits() {
        // Given two identical append edits.
        let content = "a\nb\n";
        let pos = anchor(2, compute_line_hash(2, "b"));
        let edits = vec![
            HashlineEdit::Append {
                pos: Some(pos.clone()),
                lines: vec!["x".to_owned()],
            },
            HashlineEdit::Append {
                pos: Some(pos),
                lines: vec!["x".to_owned()],
            },
        ];

        // When applying.
        let result = apply_hashline_edits(content, &edits).expect("should apply");

        // Then only one copy is applied (deduplication).
        assert_eq!(result.content, "a\nb\nx\n");
    }

    #[rstest::rstest]
    fn conflict_insert_inside_replace() {
        // Given a replace spanning lines 1-3 and a prepend at line 2 (inside the range).
        let content = "a\nb\nc\nd\n";
        let pos1 = anchor(1, compute_line_hash(1, "a"));
        let end1 = anchor(3, compute_line_hash(3, "c"));
        let pos2 = anchor(2, compute_line_hash(2, "b"));
        let edits = vec![
            HashlineEdit::Replace {
                pos: pos1,
                end: Some(end1),
                lines: vec!["X".to_owned()],
            },
            HashlineEdit::Prepend {
                pos: Some(pos2),
                lines: vec!["Y".to_owned()],
            },
        ];

        // When applying.
        let result = apply_hashline_edits(content, &edits);

        // Then it fails with a conflict error.
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("E_EDIT_CONFLICT"));
    }

    #[rstest::rstest]
    fn overlapping_replaces_rejected() {
        // Given two replace edits that overlap.
        let content = "a\nb\nc\n";
        let tag1 = anchor(1, compute_line_hash(1, "a"));
        let tag2 = anchor(2, compute_line_hash(2, "b"));
        let tag3 = anchor(3, compute_line_hash(3, "c"));
        let edits = vec![
            HashlineEdit::Replace {
                pos: tag1,
                end: Some(tag2.clone()),
                lines: vec!["X".to_owned()],
            },
            HashlineEdit::Replace {
                pos: tag2,
                end: Some(tag3),
                lines: vec!["Y".to_owned()],
            },
        ];

        // When applying.
        let result = apply_hashline_edits(content, &edits);

        // Then it fails with a conflict error.
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("E_EDIT_CONFLICT"));
    }
    fn compute_line_hash(line_num: usize, line: &str) -> String {
        crate::feat::tools_actor::edit::hash::compute_line_hash(line_num, line).to_owned()
    }
}
