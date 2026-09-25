//! The skill picker's state cell payload and its entry type.
//!
//! The skill picker is owned entirely by the skills slice: this type is what
//! its cell holds, so the kernel never names a field for it. The cell is the
//! picker's selection state plus the private snapshot taken when the picker
//! opened, which lets ESC restore exactly the set of enabled skills that was
//! in force before any toggles.

use std::collections::HashSet;

use crate::skill::SkillSource;
use jinn_picker::RowCtx;
use jinn_slices::SlotKey;
use jinn_theme::Theme;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

/// The cell slot holding the skill picker's state.
#[must_use]
pub fn skill_picker_slot() -> SlotKey {
    SlotKey::builtin("skills", "picker")
}

/// Result rows assumed before the render pass has measured the real popup.
///
/// Chosen to match the kernel's pre-measurement fallback, so the first keypress
/// after opening the picker pages the same way it always has.
pub const RESULTS_VIEWPORT_FALLBACK: usize = 20;

/// The skill picker's complete state: what it shows, and the set it restores
/// to on cancel.
#[derive(Debug)]
pub struct SkillPickerState {
    /// The selection/filter state backing the picker's rows.
    pub selection: jinn_selection_widget::SelectionState<jinn_picker::PickerEntry<SkillEntry>>,
    /// The disabled-skill set captured when the picker opened, or `None`
    /// before the first open. ESC restores it; confirm commits the toggled
    /// set instead.
    pub snapshot: Option<HashSet<String>>,
    /// The preview pane's scroll offset for the highlighted skill.
    ///
    /// Lives beside the selection because the preview follows the cursor:
    /// highlighting a different skill resets it.
    pub preview_scroll: usize,
    /// The picker's rendered-preview cache, shared with the render pass.
    pub preview_cache: std::sync::Arc<crate::skill_preview_cache::SkillPreviewCache>,
    /// How many result rows fit on screen, measured by the render pass.
    ///
    /// Paging needs a real row count: `SelectionState`'s `max_visible` argument
    /// decides whether the scroll window follows the cursor, so passing a
    /// constant would let the highlight walk off-screen. The kernel used to
    /// publish this measurement into its own state every frame; a slice-owned
    /// picker measures it in its own renderer instead, which is the same
    /// information arriving by a slice-owned route.
    pub results_viewport: usize,
    /// The theme the rows were built with, so a background repaint recolors
    /// them consistently instead of stranding them on a stale palette.
    pub theme: jinn_theme::Theme,
}

impl Default for SkillPickerState {
    fn default() -> Self {
        Self {
            selection: jinn_selection_widget::SelectionState::new(),
            snapshot: None,
            preview_scroll: 0,
            preview_cache: std::sync::Arc::new(crate::skill_preview_cache::SkillPreviewCache::new()),
            // Matches the kernel's pre-measurement fallback, so the very first
            // keypress before any render behaves as it always has.
            results_viewport: RESULTS_VIEWPORT_FALLBACK,
            theme: jinn_theme::default_theme(),
        }
    }
}

impl SkillPickerState {
    /// Clears the filter and preview scroll, ready for a fresh open.
    ///
    /// The snapshot survives: it is the set ESC restores to, and a reopen
    /// re-captures it only once the user confirms.
    pub fn reset(&mut self) {
        self.selection.clear_filter();
        self.selection.move_up(usize::MAX);
        self.preview_scroll = 0;
    }
}

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
#[must_use]
pub fn body_hash_key(body: &str) -> String {
    use std::hash::Hasher as _;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    hasher.write(body.as_bytes());
    hasher.finish().to_string()
}

/// Renders one skill picker row through the picker framework's row hook.
///
/// The single renderer both the skill spec and the slice's entry writer
/// supply, so the enabled marker, project badge, and name highlighting cannot
/// drift between the two wrapping paths.
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

    fn render_row(&self, _is_selected: bool) -> Line<'static> {
        // Rows render through the spec's row hook via PickerEntry; this
        // impl only supplies tree structure (id/parent_id) and filter text.
        Line::raw(self.display_label().to_owned())
    }

    fn render_row_with_highlight(
        &self,
        _is_selected: bool,
        _match_indices: &[std::ops::Range<usize>],
    ) -> Line<'static> {
        Line::raw(self.display_label().to_owned())
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
