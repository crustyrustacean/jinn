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

/// Stable cache key for a skill body: the decimal content hash.
///
/// Keyed on body content (not name) so that editing a SKILL.md or a project
/// skill shadowing a global of the same name produces a distinct cache entry
/// — the render cache never serves the wrong markdown. The skill spec's
/// `.preview_key` hook delegates here.
pub fn body_hash_key(body: &str) -> String {
    use std::hash::Hasher as _;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    hasher.write(body.as_bytes());
    hasher.finish().to_string()
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
    crate::feat::ui::chat_log::markdown::render_markdown(
        &entry.body,
        ctx.width as u16,
        &entry.theme,
    )
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
}
