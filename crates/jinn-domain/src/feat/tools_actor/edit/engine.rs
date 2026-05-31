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
    assert_no_display_prefixes, compute_line_hash, format_tag, parse_anchor, Anchor,
    HashMismatch,
};

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
    ReplaceText {
        old_text: String,
        new_text: String,
    },
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

/// A resolved byte span for applying an edit.
#[derive(Debug, Clone)]
struct ResolvedSpan {
    /// "replace" or "insert".
    kind: &'static str,
    /// Edit index in the original request.
    index: usize,
    /// Human-readable label for error messages.
    label: String,
    /// Start byte offset (inclusive).
    start: usize,
    /// End byte offset (exclusive).
    end: usize,
    /// The replacement text.
    replacement: String,
    /// For inserts: the insertion boundary (for ordering/conflict detection).
    boundary: Option<usize>,
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
                let pos_str = edit.pos.as_deref().ok_or_else(|| {
                    "[E_BAD_OP] Replace requires a \"pos\" anchor.".to_owned()
                })?;
                let pos = parse_anchor(pos_str)?;
                let end = edit
                    .end
                    .as_deref()
                    .map(parse_anchor)
                    .transpose()?;
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
                let pos = edit
                    .pos
                    .as_deref()
                    .map(parse_anchor)
                    .transpose()?;
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
                let pos = edit
                    .pos
                    .as_deref()
                    .map(parse_anchor)
                    .transpose()?;
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
                    return Err(
                        "[E_BAD_OP] replace_text requires non-empty oldText.".to_owned(),
                    );
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
pub fn validate_anchors(
    edits: &[HashlineEdit],
    file_lines: &[&str],
) -> Vec<HashMismatch> {
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
                            actual: format!(
                                "(range start {} > end {})",
                                pos.line, end_anchor.line
                            ),
                        });
                        continue;
                    }
                    validate_anchor(end_anchor, file_lines, &mut mismatches, &mut seen_mismatch_lines);
                }
            }
            HashlineEdit::Append { pos, .. } | HashlineEdit::Prepend { pos, .. } => {
                if let Some(anchor) = pos {
                    validate_anchor(anchor, file_lines, &mut mismatches, &mut seen_mismatch_lines);
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
    let actual = compute_line_hash(anchor.line, file_lines[anchor.line - 1]);
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
pub fn format_mismatch_error(
    mismatches: &[HashMismatch],
    file_lines: &[&str],
) -> String {
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
    out.push_str(&format!(
        "[E_STALE_ANCHOR] {n} stale anchor{}. Retry with the >>> LINE#HASH lines below; keep both endpoints for range replaces.\n\n",
        if n > 1 { "s" } else { "" }
    ));

    let mut prev = 0usize;
    for num in sorted {
        if num == 0 || num > file_lines.len() {
            continue;
        }
        if prev != 0 && num > prev + 1 {
            out.push_str("    ...\n");
        }
        prev = num;

        let content = file_lines[num - 1];
        let h = compute_line_hash(num, content);
        if mismatch_lines.contains(&num) {
            out.push_str(&format!(
                ">>> {num:>width$}#{h}|{content}\n",
                width = width
            ));
        } else {
            out.push_str(&format!(
                "    {num:>width$}#{h}|{content}\n",
                width = width
            ));
        }
    }

    out
}

// ─── Application ────────────────────────────────────────────────────────

/// Applies validated hashline edits to the file content.
///
/// Returns the modified content, changed line range, warnings, and NOOP edits.
pub fn apply_hashline_edits(
    content: &str,
    edits: &[HashlineEdit],
) -> Result<EditResult, String> {
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

    // Build byte-offset index for each line start
    let mut line_starts: Vec<usize> = Vec::with_capacity(file_lines.len());
    let mut offset = 0;
    for (i, line) in file_lines.iter().enumerate() {
        line_starts.push(offset);
        offset += line.len();
        if i < file_lines.len() - 1 {
            offset += 1; // newline
        }
    }

    let mut warnings = Vec::new();
    let mut noop_edits = Vec::new();
    let mut spans = Vec::new();

    for (index, edit) in edits.iter().enumerate() {
        let span = resolve_edit_to_span(
            edit,
            index,
            content,
            &file_lines,
            &line_starts,
            has_terminal_newline,
            &mut noop_edits,
        )?;

        let Some(span) = span else {
            continue;
        };

        // Check for boundary duplication
        check_boundary_duplication(edit, &file_lines, &mut warnings);

        spans.push(span);
    }

    // Deduplicate identical spans
    let mut seen_keys = std::collections::HashSet::new();
    spans.retain(|s| {
        let key = format!("{}:{}:{}:{}", s.kind, s.start, s.end, s.replacement);
        seen_keys.insert(key)
    });

    // Conflict detection
    assert_no_conflicting_spans(&spans)?;

    // Sort bottom-up: highest end first, then by kind, then by index
    spans.sort_by(|a, b| {
        if b.end != a.end {
            b.end.cmp(&a.end)
        } else if a.kind != b.kind {
            if a.kind == "replace" {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Greater
            }
        } else {
            a.index.cmp(&b.index)
        }
    });

    // Apply spans
    let mut result = content.to_owned();
    for span in &spans {
        result = format!(
            "{}{}{}",
            &result[..span.start],
            span.replacement,
            &result[span.end..]
        );
    }

    // Compute changed range
    let changed_range = compute_changed_line_range(content, &result);

    Ok(EditResult {
        content: result,
        first_changed_line: changed_range.0,
        last_changed_line: changed_range.1,
        warnings,
        noop_edits,
    })
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
    match edit {
        HashlineEdit::Replace { pos, end, lines } => {
            let start_line = pos.line;
            let end_line = end.as_ref().map_or(start_line, |a| a.line);

            // NOOP check
            let original: Vec<&str> = file_lines
                [start_line - 1..end_line.min(file_lines.len())]
                .to_vec();
            if original.len() == lines.len()
                && original
                    .iter()
                    .zip(lines.iter())
                    .all(|(a, b)| a == b)
            {
                noop_edits.push(NoopEdit {
                    edit_index: index,
                    loc: format!("{}#{}", pos.line, pos.hash),
                    current_content: original.join("\n"),
                });
                return Ok(None);
            }

            let start_byte = line_starts[start_line - 1];
            let end_byte = line_starts[end_line - 1] + file_lines[end_line - 1].len();
            let replacement = lines.join("\n");

            Ok(Some(ResolvedSpan {
                kind: "replace",
                index,
                label: describe_edit(edit),
                start: start_byte,
                end: end_byte,
                replacement,
                boundary: None,
            }))
        }
        HashlineEdit::Append { pos, lines } => {
            if lines.is_empty() {
                // NOOP: inserting nothing
                noop_edits.push(NoopEdit {
                    edit_index: index,
                    loc: pos
                        .as_ref()
                        .map_or("EOF".to_owned(), |a| format!("{}#{}", a.line, a.hash)),
                    current_content: String::new(),
                });
                return Ok(None);
            }

            let inserted_text = lines.join("\n");
            if content.is_empty() {
                return Ok(Some(ResolvedSpan {
                    kind: "insert",
                    index,
                    label: describe_edit(edit),
                    start: 0,
                    end: 0,
                    replacement: inserted_text,
                    boundary: Some(0),
                }));
            }

            let Some(anchor) = pos else {
                // Append at EOF
                let replacement = if has_terminal_newline {
                    format!("{inserted_text}\n")
                } else {
                    format!("\n{inserted_text}")
                };
                return Ok(Some(ResolvedSpan {
                    kind: "insert",
                    index,
                    label: describe_edit(edit),
                    start: content.len(),
                    end: content.len(),
                    replacement,
                    boundary: Some(file_lines.len()),
                }));
            };

            let is_sentinel = has_terminal_newline && anchor.line == file_lines.len();
            let insert_pos = if is_sentinel {
                content.len()
            } else {
                line_starts[anchor.line - 1] + file_lines[anchor.line - 1].len()
            };
            let replacement = if is_sentinel {
                format!("{inserted_text}\n")
            } else {
                format!("\n{inserted_text}")
            };

            Ok(Some(ResolvedSpan {
                kind: "insert",
                index,
                label: describe_edit(edit),
                start: insert_pos,
                end: insert_pos,
                replacement,
                boundary: Some(anchor.line),
            }))
        }
        HashlineEdit::Prepend { pos, lines } => {
            if lines.is_empty() {
                noop_edits.push(NoopEdit {
                    edit_index: index,
                    loc: pos
                        .as_ref()
                        .map_or("BOF".to_owned(), |a| format!("{}#{}", a.line, a.hash)),
                    current_content: String::new(),
                });
                return Ok(None);
            }

            let inserted_text = lines.join("\n");
            let insert_pos = pos.as_ref().map_or(0, |a| line_starts[a.line - 1]);
            let replacement = if content.is_empty() {
                inserted_text
            } else {
                format!("{inserted_text}\n")
            };

            Ok(Some(ResolvedSpan {
                kind: "insert",
                index,
                label: describe_edit(edit),
                start: insert_pos,
                end: insert_pos,
                replacement,
                boundary: pos.as_ref().map(|a| a.line.saturating_sub(1)),
            }))
        }
        HashlineEdit::ReplaceText { old_text, new_text } => {
            if old_text == new_text {
                noop_edits.push(NoopEdit {
                    edit_index: index,
                    loc: format!("replace_text \"{old_text}\""),
                    current_content: old_text.clone(),
                });
                return Ok(None);
            }

            let found = find_exact_unique_match(content, old_text);
            match found {
                None => {
                    // Check if text doesn't exist at all vs. exists multiple times
                    if !content.contains(old_text.as_str()) {
                        return Err(format!(
                            "[E_NOT_FOUND] replace_text: \"{old_text}\" not found in file."
                        ));
                    }
                    return Err(format!(
                        "[E_NOT_UNIQUE] replace_text: \"{old_text}\" appears more than once."
                    ));
                }
                Some((start, end)) => Ok(Some(ResolvedSpan {
                    kind: "replace",
                    index,
                    label: describe_edit(edit),
                    start,
                    end,
                    replacement: new_text.clone(),
                    boundary: None,
                })),
            }
        }
    }
}

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
        let Some(idx) = content[from..].find(old_text) else {
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
        && last_repl
            .chars()
            .any(|c| c.is_alphanumeric())
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
        HashlineEdit::Append { pos, .. } => {
            pos.as_ref().map_or("append at EOF".to_owned(), |a| {
                format!("append after {}#{}", a.line, a.hash)
            })
        }
        HashlineEdit::Prepend { pos, .. } => {
            pos.as_ref().map_or("prepend at BOF".to_owned(), |a| {
                format!("prepend before {}#{}", a.line, a.hash)
            })
        }
        HashlineEdit::ReplaceText { old_text, .. } => {
            let preview = if old_text.len() > 32 {
                format!("{}...", &old_text[..29])
            } else {
                old_text.replace('\n', "\\n")
            };
            format!("replace_text \"{preview}\"")
        }
    }
}

/// Rejects overlapping spans and same-boundary inserts.
fn assert_no_conflicting_spans(spans: &[ResolvedSpan]) -> Result<(), String> {
    for i in 0..spans.len() {
        for j in (i + 1)..spans.len() {
            let left = &spans[i];
            let right = &spans[j];

            // Two inserts at same boundary
            if left.kind == "insert" && right.kind == "insert" {
                if left.boundary == right.boundary {
                    return Err(format!(
                        "[E_EDIT_CONFLICT] Conflicting edits in a single request: edit {} ({}) and edit {} ({}) target the same insertion boundary.",
                        left.index, left.label, right.index, right.label
                    ));
                }
                continue;
            }

            // Two replaces that overlap
            if left.kind == "replace" && right.kind == "replace" {
                if left.start < right.end && right.start < left.end {
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
            if insert_span.start >= replace_span.start
                && insert_span.start < replace_span.end
            {
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
#[allow(clippy::items_after_statements)]
fn compute_changed_line_range(
    original: &str,
    result: &str,
) -> (Option<usize>, Option<usize>) {
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
    while first_diff < min_len && original.as_bytes()[first_diff] == result.as_bytes()[first_diff] {
        first_diff += 1;
    }

    // Find last differing byte
    let mut last_orig = original.len() as isize - 1;
    let mut last_res = result.len() as isize - 1;
    while last_orig >= first_diff as isize
        && last_res >= first_diff as isize
        && original.as_bytes()[last_orig as usize]
            == result.as_bytes()[last_res as usize]
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

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
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
                pos: tag1.clone(),
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

    fn compute_line_hash(line_num: usize, line: &str) -> String {
        crate::feat::tools_actor::edit::hash::compute_line_hash(line_num, line).to_owned()
    }
}
