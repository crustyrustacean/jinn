//! MCP server picker entry type.

use crate::McpConnectionStatus;
use jinn_theme::Theme;

/// An MCP server entry ready for display in the MCP inspector.
///
/// Mirrors the tool picker entry: a name plus a dim description, with a ✓/✗
/// marker showing the per-session enabled state. Rendering lives in the picker
/// spec; this type is pure data.
#[derive(Debug, Clone)]
pub struct McpServerEntry {
    /// Server name (the `[[mcp_server]].name`, unique per `jinn.toml`).
    pub name: String,
    /// Human-readable launch summary (e.g. `"npx @excalimate/mcp-server"`).
    pub description: String,
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
        Self {
            name,
            description,
            enabled,
            theme,
            status: None,
            stderr_tail: String::new(),
            tools: Vec::new(),
            preview_mode: McpPreviewMode::default(),
        }
    }
}

impl jinn_selection_widget::TreeItem for McpServerEntry {
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
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use super::*;
    use jinn_theme::default_theme;

    fn make_entry(name: &str, description: &str, enabled: bool) -> McpServerEntry {
        McpServerEntry::new(
            name.to_owned(),
            description.to_owned(),
            enabled,
            default_theme(),
        )
    }

    #[rstest::rstest]
    fn new_entry_defaults_to_logs_preview_mode() {
        // Given a freshly built entry.
        let entry = make_entry("excalimate", "npx ...", true);

        // When reading its preview mode.
        // Then it defaults to the logs pane.
        assert_eq!(entry.preview_mode, McpPreviewMode::Logs);
    }
}
