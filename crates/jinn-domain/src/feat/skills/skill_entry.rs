//! Skill picker entry type and rendering.

use jinn_picker::RowCtx;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use jinn_skills_msg::SkillSource;
use jinn_theme::Theme;

/// A skill entry ready for display in the skill picker.
#[derive(Debug, Clone)]
pub struct SkillEntry {
    /// Skill name (unique identifier, e.g., "phased-task-loop", "web-coder").
    pub name: String,
    /// Human-readable skill description.
    pub description: String,
    /// Markdown body content (from SKILL.md, after stripping frontmatter).
    pub body: String,
    /// Whether the skill is currently enabled for this session.
    pub enabled: bool,
    /// Where this skill was discovered from (global vs project).
    pub source: SkillSource,
    /// Theme for styling.
    pub theme: Theme,
}

/// Strong cache key for a skill body: the decimal content hash.
///
/// Keyed on body content (not name) so that editing a SKILL.md or a project
/// skill shadowing a global of the same name produces a distinct cache entry
/// — the render cache never serves the wrong markdown. Because the hash reads
/// every byte, it costs O(body size), so it is NOT used on the per-frame render
/// path; see [`body_signature`] for that.
pub fn body_hash_key(body: &str) -> String {
    use std::hash::Hasher as _;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    hasher.write(body.as_bytes());
    hasher.finish().to_string()
}

/// Per-frame cache key for a skill body: the decimal byte length.
///
/// This is the key the skill spec's `.preview_key` hook supplies, because it
/// resolves in O(1) regardless of how large the body is. Skill bodies run to
/// tens of kilobytes, so hashing the whole body on every render frame was a
/// constant per-frame cost for as long as the picker was open.
///
/// Known blind spot: two different bodies of identical byte length share a
/// key, so a body rewritten in place to the same length would reuse a stale
/// render. Any append or deletion changes the length and misses correctly,
/// which covers the normal editing pattern for a SKILL.md.
pub fn body_signature(body: &str) -> String {
    body.len().to_string()
}

/// Renders one skill picker row through the picker framework's row hook.
///
/// The single renderer both the skill spec and the kernel's entry writer supply,
/// so the enabled marker, project badge, and name highlighting cannot drift
/// between the two wrapping paths.
pub fn skill_row(entry: &SkillEntry, ctx: &RowCtx<'_>) -> Line<'static> {
    let style = if ctx.is_selected {
        Style::default()
            .fg(entry.theme.primary_text)
            .bg(entry.theme.picker_selected_bg)
    } else {
        Style::default()
    };

    let (marker, marker_color) = if entry.enabled {
        ("\u{2713} ", entry.theme.focus_accent) // ✓
    } else {
        ("\u{2717} ", entry.theme.error_text) // ✗
    };

    let marker_span = Span::styled(marker.to_owned(), Style::default().fg(marker_color));

    if ctx.match_ranges.is_empty() {
        let name_span = Span::styled(entry.name.clone(), style);
        let mut spans = vec![marker_span, name_span];
        if let Some(badge) = project_badge_span(entry) {
            spans.push(badge);
        }
        return Line::from(spans);
    }

    // Match indices are byte offsets into search_text = "{name} {description}".
    // Only highlight the name portion in the row (description is in the preview pane).
    let name_indices = split_match_indices(ctx.match_ranges, entry.name.len());

    let name_spans = jinn_selection_widget::highlight::highlight_text_with_bg(
        &entry.name,
        style,
        &name_indices,
        entry.theme.picker_highlight_bg,
    );

    let mut spans = vec![marker_span];
    spans.extend(name_spans);
    if let Some(badge) = project_badge_span(entry) {
        spans.push(badge);
    }
    Line::from(spans)
}

/// Badge span indicating project-scoped provenance, if applicable.
///
/// Appended to the row after the skill name. Global skills render no badge.
fn project_badge_span(entry: &SkillEntry) -> Option<Span<'static>> {
    match &entry.source {
        SkillSource::Project { .. } => Some(Span::styled(
            " (project)".to_owned(),
            Style::default().fg(entry.theme.muted_text),
        )),
        SkillSource::Global => None,
    }
}

/// Renders the skill's markdown body for the preview pane.
pub fn render_skill_preview(
    entry: &SkillEntry,
    ctx: &jinn_picker::PreviewCtx<'_>,
) -> Vec<Line<'static>> {
    if entry.body.is_empty() {
        return Vec::new();
    }
    jinn_chat_log_view::chat_log::render_markdown(&entry.body, ctx.width as u16, &entry.theme)
}

/// Splits match indices from `search_text = "{name} {description}"` into
/// name-portion ranges, clamped to the name's byte length. Description
/// indices are dropped — the row highlights the name only.
fn split_match_indices(
    indices: &[std::ops::Range<usize>],
    name_len: usize,
) -> Vec<std::ops::Range<usize>> {
    indices
        .iter()
        .filter(|range| range.start < name_len)
        .map(|range| range.start..range.end.min(name_len))
        .collect()
}

impl jinn_selection_widget::TreeItem for SkillEntry {
    fn id(&self) -> &str {
        &self.name
    }

    fn parent_id(&self) -> Option<&str> {
        None
    }

    fn display_label(&self) -> &str {
        &self.name
    }

    fn render_row(&self, _is_selected: bool) -> ratatui::text::Line<'static> {
        // Rows render through the spec's row hook via PickerEntry; this
        // impl only supplies tree structure (id/parent_id) and filter text.
        ratatui::text::Line::raw(self.display_label().to_owned())
    }

    fn render_row_with_highlight(
        &self,
        _is_selected: bool,
        _match_indices: &[std::ops::Range<usize>],
    ) -> ratatui::text::Line<'static> {
        ratatui::text::Line::raw(self.display_label().to_owned())
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        clippy::single_range_in_vec_init,
        reason = "test code"
    )]
    use super::*;

    #[rstest::rstest]
    fn body_hash_key_distinguishes_body_not_name() {
        // Given two entries with the same name but different bodies, and a
        // third with a different name but the same body as the first.
        let a = body_hash_key("# body one");
        let b = body_hash_key("# body two");
        let c = body_hash_key("# body one");

        // When hashing the bodies.
        // Then same body -> same key, different body -> different key,
        // regardless of skill name.
        assert_eq!(a, c);
        assert_ne!(a, b);
    }

    #[rstest::rstest]
    fn body_signature_distinguishes_appended_and_deleted_bodies() {
        // Given a body and two edits that change its length.
        let original = body_signature("# body");
        let appended = body_signature("# body plus more");
        let shortened = body_signature("# bod");

        // When appending and deleting change the byte length.
        // Then the signature changes, so the cache misses and never serves a
        // stale render for an edited body.
        assert_ne!(original, appended);
        assert_ne!(original, shortened);
    }

    #[rstest::rstest]
    fn body_signature_collides_for_distinct_bodies_of_equal_length() {
        // Given two different bodies that happen to be the same byte length.
        let alpha = body_signature("# alpha");
        let beta = body_signature("# bravo");

        // When they are the same size on disk.
        // Then they share a signature. This is the documented blind spot of
        // using length as the per-frame key: an in-place rewrite to the exact
        // same byte length is invisible to the cache.
        assert_eq!(alpha.len(), beta.len());
        assert_eq!(alpha, beta);
    }

    #[rstest::rstest]
    fn body_signature_is_far_cheaper_than_hashing_a_large_body() {
        // Given a body the size of a real skill file (~27 KB).
        let body = "# Rust rules\n\nlet x = 1;\n".repeat(1024);
        assert!(body.len() > 20_000, "test body should be realistically large");

        // When computing the per-frame key many times versus hashing it.
        let iters = 2_000;
        let cheap = std::time::Instant::now();
        for _ in 0..iters {
            std::hint::black_box(body_signature(std::hint::black_box(&body)));
        }
        let cheap_elapsed = cheap.elapsed();

        let expensive = std::time::Instant::now();
        for _ in 0..iters {
            std::hint::black_box(body_hash_key(std::hint::black_box(&body)));
        }
        let expensive_elapsed = expensive.elapsed();

        // Then the signature costs a small fraction of a full hash, so the
        // per-frame key is effectively free compared to hashing the body.
        assert!(
            cheap_elapsed * 50 < expensive_elapsed,
            "signature {cheap_elapsed:?} should be far cheaper than hash {expensive_elapsed:?}"
        );
    }
}
