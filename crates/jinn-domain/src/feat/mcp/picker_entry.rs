//! MCP server picker entry type and rendering.

use crate::feat::theme::Theme;
use jinn_selection_widget::PickerItem;
use jinn_selection_widget::highlight::highlight_text_with_bg;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use std::ops::Range;

/// An MCP server entry ready for display in the MCP picker.
///
/// Mirrors [`ToolEntry`](crate::feat::tools_actor::tool_entry::ToolEntry): a
/// name plus a dim description, with a ✓/✗ marker showing the per-session
/// enabled state.
#[derive(Debug, Clone)]
pub struct McpServerEntry {
    /// Server name (the `[[mcp_servers]].name`, unique per `jinn.toml`).
    pub name: String,
    /// Human-readable launch summary (e.g. `"npx @excalimate/mcp-server"`).
    pub description: String,
    /// Combined searchable text: `"{name} {description}"`.
    /// Used for fuzzy matching so users can search by description terms.
    pub search_text: String,
    /// Whether this server is enabled for the active session.
    pub enabled: bool,
    /// Theme for styling.
    pub theme: Theme,
}

impl McpServerEntry {
    /// Builds an entry from a server name, launch description, and enabled flag.
    #[must_use]
    pub fn new(name: String, description: String, enabled: bool, theme: Theme) -> Self {
        let search_text = format!("{name} {description}");
        Self {
            name,
            description,
            search_text,
            enabled,
            theme,
        }
    }
}

impl PickerItem for McpServerEntry {
    fn display_label(&self) -> &str {
        &self.search_text
    }

    fn render_row(&self, is_selected: bool) -> Line<'static> {
        render_entry(self, is_selected, None)
    }

    fn render_row_with_highlight(
        &self,
        is_selected: bool,
        match_indices: &[Range<usize>],
    ) -> Line<'static> {
        render_entry(self, is_selected, Some(match_indices))
    }
}

/// Renders a row either plain or with filter-highlight applied.
fn render_entry(
    entry: &McpServerEntry,
    is_selected: bool,
    match_indices: Option<&[Range<usize>]>,
) -> Line<'static> {
    let style = if is_selected {
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

    // Description always dimmed; matches the tool picker's em-dash layout.
    let desc_style = crate::feat::picker::style::dim_style(is_selected, &entry.theme);

    match match_indices {
        None => {
            let name_span = Span::styled(entry.name.clone(), style);
            let desc_span = Span::styled(format!(" \u{2014} {}", entry.description), desc_style);
            Line::from(vec![marker_span, name_span, desc_span])
        }
        Some(indices) => {
            let (name_indices, desc_indices) = split_match_indices(indices, entry.name.len());
            let mut spans = vec![marker_span];
            spans.extend(highlight_text_with_bg(
                &entry.name,
                style,
                &name_indices,
                entry.theme.picker_highlight_bg,
            ));
            spans.push(Span::styled(" \u{2014} ".to_owned(), desc_style));
            spans.extend(highlight_text_with_bg(
                &entry.description,
                desc_style,
                &desc_indices,
                entry.theme.picker_highlight_bg,
            ));
            Line::from(spans)
        }
    }
}

/// Splits match indices from `search_text = "{name} {description}"` into
/// name-portion and description-portion indices (relative to each substring).
fn split_match_indices(
    indices: &[Range<usize>],
    name_len: usize,
) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    let desc_offset = name_len + 1;
    let mut name_indices = Vec::new();
    let mut desc_indices = Vec::new();
    for range in indices {
        if range.start < name_len {
            let end = range.end.min(name_len);
            name_indices.push(range.start..end);
        }
        if range.end > desc_offset {
            let start = range.start.saturating_sub(desc_offset);
            let end = range.end.saturating_sub(desc_offset);
            desc_indices.push(start..end);
        }
    }
    (name_indices, desc_indices)
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing,
        clippy::single_range_in_vec_init,
        reason = "test code"
    )]
    use super::*;
    use crate::feat::theme::default_theme;

    fn make_entry(name: &str, description: &str, enabled: bool) -> McpServerEntry {
        McpServerEntry::new(
            name.to_owned(),
            description.to_owned(),
            enabled,
            default_theme(),
        )
    }

    #[rstest::rstest]
    fn display_label_combines_name_and_description() {
        // Given an entry with a name and launch description.
        let entry = make_entry("excalimate", "npx @excalimate/mcp-server", true);

        // When getting the display label.
        let label = entry.display_label();

        // Then it contains both name and description.
        assert_eq!(label, "excalimate npx @excalimate/mcp-server");
    }

    #[rstest::rstest]
    fn render_row_enabled_shows_checkmark() {
        let entry = make_entry("excalimate", "npx ...", true);
        let rendered = entry.render_row(false).to_string();
        assert!(
            rendered.contains('\u{2713}'),
            "enabled should show \u{2713}"
        );
        assert!(
            !rendered.contains('\u{2717}'),
            "enabled should not show \u{2717}"
        );
    }

    #[rstest::rstest]
    fn render_row_disabled_shows_cross() {
        let entry = make_entry("excalimate", "npx ...", false);
        let rendered = entry.render_row(false).to_string();
        assert!(
            rendered.contains('\u{2717}'),
            "disabled should show \u{2717}"
        );
        assert!(
            !rendered.contains('\u{2713}'),
            "disabled should not show \u{2713}"
        );
    }

    #[rstest::rstest]
    fn render_row_shows_description_after_em_dash() {
        // Given an entry with a description.
        let entry = make_entry("excalimate", "npx @excalimate/mcp-server", true);

        // When rendering the row.
        let rendered = entry.render_row(false).to_string();

        // Then the description appears after an em-dash.
        assert!(rendered.contains('\u{2014}'), "should contain em-dash");
        assert!(rendered.contains("npx @excalimate/mcp-server"));
    }

    #[rstest::rstest]
    fn split_match_indices_partitions_across_boundary() {
        // search_text = "abc xyz" (name="abc" len 3, separator at 3, desc="xyz").
        let (name_idx, desc_idx) = split_match_indices(&[0..5], 3);
        assert_eq!(name_idx, vec![0..3]);
        assert_eq!(desc_idx, vec![0..1]);
    }
}
