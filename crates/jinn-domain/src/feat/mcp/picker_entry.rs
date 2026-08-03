//! MCP server picker entry type and rendering.

use crate::feat::mcp_actor::protocol::McpConnectionStatus;
use crate::feat::theme::Theme;
use jinn_provider::ToolDefinition;
use jinn_selection_widget::PickerItem;
use jinn_selection_widget::PreviewContent;
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
    /// Server name (the `[[mcp_server]].name`, unique per `jinn.toml`).
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
    /// Live connection status (Starting/Running/Dead) for the preview's
    /// status badge. `None` when disabled or not yet seen.
    pub status: Option<McpConnectionStatus>,
    /// Captured stderr tail for the logs preview pane.
    pub stderr_tail: String,
    /// Tools advertised by this server, namespaced + stripped to
    /// `(local_name, description)` pairs for the tools preview pane.
    pub tools: Vec<(String, String)>,
    /// Which preview pane is shown: logs (status + stderr) or tools.
    pub preview_mode: McpPreviewMode,
}

/// Toggles the MCP server preview pane between logs and tools.
///
/// Defaults to [`McpPreviewMode::Logs`] so the user sees server health
/// (status badge + stderr) first; they flip to [`McpPreviewMode::Tools`]
/// to inspect the advertised tools.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum McpPreviewMode {
    /// Status badge + live stderr tail.
    #[default]
    Logs,
    /// One line per advertised tool (`name — description`).
    Tools,
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
            status: None,
            stderr_tail: String::new(),
            tools: Vec::new(),
            preview_mode: McpPreviewMode::default(),
        }
    }
}

impl PreviewContent for McpServerEntry {
    fn preview_lines(&self, width: usize) -> Vec<Line<'static>> {
        match self.preview_mode {
            McpPreviewMode::Logs => self.logs_preview(width),
            McpPreviewMode::Tools => self.tools_preview(),
        }
    }
    // Live tail/stderr/tools refresh every frame — never cache.
    fn cache_key(&self) -> Option<String> {
        None
    }
}

impl McpServerEntry {
    /// Logs pane: a status badge line followed by the stderr tail,
    /// soft-wrapped so long lines don't overflow the preview width.
    fn logs_preview(&self, width: usize) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        lines.push(self.status_badge_line());
        if self.stderr_tail.trim().is_empty() {
            lines.push(
                Line::from("(no stderr yet)".to_owned())
                    .style(Style::default().fg(self.theme.muted_text)),
            );
        } else {
            for raw in self.stderr_tail.lines() {
                lines.extend(wrap_line(raw, width, self.theme.primary_text));
            }
        }
        lines
    }

    /// Tools pane: one line per advertised tool (`name — description`).
    fn tools_preview(&self) -> Vec<Line<'static>> {
        if self.tools.is_empty() {
            return vec![
                Line::from("(no tools advertised)".to_owned())
                    .style(Style::default().fg(self.theme.muted_text)),
            ];
        }
        self.tools
            .iter()
            .map(|(name, desc)| {
                Line::from(vec![
                    Span::styled(name.clone(), Style::default().fg(self.theme.primary_text)),
                    Span::styled(
                        format!(" \u{2014} {desc}"),
                        Style::default().fg(self.theme.muted_text),
                    ),
                ])
            })
            .collect()
    }

    /// One styled line: `Status: running` colored by the live state.
    fn status_badge_line(&self) -> Line<'static> {
        let (label, color) = match self.status {
            None => ("disabled", self.theme.muted_text),
            Some(McpConnectionStatus::Starting) => ("starting", ratatui::style::Color::Yellow),
            Some(McpConnectionStatus::Running) => ("running", ratatui::style::Color::Green),
            Some(McpConnectionStatus::Dead) => ("dead", ratatui::style::Color::Red),
        };
        Line::from(vec![
            Span::styled(
                "Status: ".to_owned(),
                Style::default().fg(self.theme.muted_text),
            ),
            Span::styled(label.to_owned(), Style::default().fg(color)),
        ])
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

/// Greedily wraps `raw` to `width` columns, returning one styled line per
/// chunk. Guards against a zero width by treating it as 1 so we never loop
/// forever on an empty/negative-pane edge case.
fn wrap_line(raw: &str, width: usize, color: ratatui::style::Color) -> Vec<Line<'static>> {
    use unicode_segmentation::UnicodeSegmentation;
    let cap = width.max(1);
    let style = Style::default().fg(color);
    let mut out = Vec::new();
    let mut buf = String::new();
    for grapheme in raw.graphemes(true) {
        if buf.graphemes(true).count() >= cap {
            out.push(Line::from(buf.clone()).style(style));
            buf.clear();
        }
        buf.push_str(grapheme);
    }
    if !buf.is_empty() || out.is_empty() {
        out.push(Line::from(buf).style(style));
    }
    out
}
/// A live snapshot of one MCP server's inspectable state, computed from the
/// active session's maps + tool definitions.
///
/// Pure helper: given read-only inputs it returns the values the preview pane
/// needs. The render path calls this each frame to refresh the selected entry
/// without mutating the stored item list.
///
/// Tools are collected by filtering `defs` for names carrying this server's
/// `mcp__<server>__` prefix, then stripping the prefix to recover the
/// server-side tool name.
#[must_use]
pub fn refresh_snapshot(
    server_name: &str,
    status: Option<McpConnectionStatus>,
    stderr_tail: &str,
    defs: &[ToolDefinition],
) -> (Option<McpConnectionStatus>, String, Vec<(String, String)>) {
    let prefix = jinn_mcp::provider_prefix(server_name);
    let tools = defs
        .iter()
        .filter(|d| d.name.starts_with(&prefix))
        .map(|d| {
            (
                d.name
                    .strip_prefix(prefix.as_str())
                    .unwrap_or(&d.name)
                    .to_owned(),
                d.description.clone(),
            )
        })
        .collect();
    (status, stderr_tail.to_owned(), tools)
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

    #[rstest::rstest]
    fn logs_preview_shows_status_badge_and_tail_lines() {
        // Given an enabled entry with a running status and a multi-line stderr tail.
        let mut entry = make_entry("excalimate", "npx ...", true);
        entry.status = Some(McpConnectionStatus::Running);
        entry.stderr_tail = "first line\nsecond line".to_owned();
        entry.preview_mode = McpPreviewMode::Logs;

        // When computing preview lines.
        let lines = entry.preview_lines(40);

        // Then the first line is the status badge.
        assert_eq!(
            lines.first().expect("at least badge").to_string(),
            "Status: running"
        );
        // And subsequent lines are the tail (one per stderr line).
        assert!(lines.len() >= 3, "badge + 2 tail lines");
        let rendered: Vec<String> = lines.iter().map(std::string::ToString::to_string).collect();
        assert!(rendered[1..].iter().any(|l| l.contains("first line")));
        assert!(rendered[1..].iter().any(|l| l.contains("second line")));
    }

    #[rstest::rstest]
    fn tools_preview_shows_one_line_per_tool() {
        // Given an entry with two advertised tools, in Tools mode.
        let mut entry = make_entry("excalimate", "npx ...", true);
        entry.tools = vec![
            ("create_scene".to_owned(), "Create a scene".to_owned()),
            ("auto_animate".to_owned(), "Auto-animate".to_owned()),
        ];
        entry.preview_mode = McpPreviewMode::Tools;

        // When computing preview lines.
        let lines = entry.preview_lines(40);

        // Then there is one line per tool.
        assert_eq!(lines.len(), 2);
        let rendered: String = lines
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains("create_scene"));
        assert!(rendered.contains("auto_animate"));
    }

    #[rstest::rstest]
    fn tools_preview_empty_shows_placeholder() {
        // Given an entry with no tools, in Tools mode.
        let mut entry = make_entry("excalimate", "npx ...", true);
        entry.preview_mode = McpPreviewMode::Tools;

        // When computing preview lines.
        let lines = entry.preview_lines(40);

        // Then a single placeholder line is shown.
        assert_eq!(lines.len(), 1);
        assert!(lines[0].to_string().contains("no tools advertised"));
    }

    fn tool_def(name: &str, desc: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.to_owned(),
            description: desc.to_owned(),
            parameters: serde_json::Value::Object(serde_json::Map::new()),
            prompt_snippet: None,
            prompt_guidelines: Vec::new(),
            server_tool_type: None,
        }
    }

    #[rstest::rstest]
    fn refresh_snapshot_filters_and_strips_prefix() {
        // Given tool defs mixing this server, another server, and a builtin.
        let defs = vec![
            tool_def("mcp__excalimate__create_scene", "Create a scene"),
            tool_def("mcp__excalimate__auto_animate", "Auto-animate"),
            tool_def("mcp__other__create_scene", "Other server"),
            tool_def("file_read", "A builtin"),
        ];

        // When refreshing the snapshot for "excalimate".
        let (_status, _stderr, tools) = refresh_snapshot("excalimate", None, "", &defs);

        // Then only excalimate's tools are collected, with prefixes stripped.
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].0, "create_scene");
        assert_eq!(tools[0].1, "Create a scene");
        assert_eq!(tools[1].0, "auto_animate");
    }

    #[rstest::rstest]
    fn refresh_snapshot_passes_status_and_stderr_through() {
        // Given a status and stderr tail.
        // When refreshing.
        let (status, stderr, _tools) =
            refresh_snapshot("srv", Some(McpConnectionStatus::Dead), "boom", &[]);

        // Then they pass through unchanged.
        assert_eq!(status, Some(McpConnectionStatus::Dead));
        assert_eq!(stderr, "boom");
    }

    #[rstest::rstest]
    fn refresh_snapshot_no_matching_tools_returns_empty() {
        // Given defs with no matching prefix.
        let defs = vec![tool_def("file_read", "builtin")];

        // When refreshing for an unknown server.
        let (_status, _stderr, tools) = refresh_snapshot("ghost", None, "", &defs);

        // Then no tools are collected.
        assert!(tools.is_empty());
    }
}
