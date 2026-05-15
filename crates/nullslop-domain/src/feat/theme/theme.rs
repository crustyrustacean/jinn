//! Theme struct — the resolved set of semantic colors used by the renderer.

use ratatui::style::Color;

use super::color::ThemeColor;
use super::default_theme;

/// Resolved theme with all semantic color fields.
///
/// Every field is a [`Color`] — fully resolved from whatever format the
/// TOML file specified. Missing TOML fields fall back to the default theme.
///
/// The theme is stored in `AppState.frontend` and read by all render sites.
#[derive(Debug, Clone)]
pub struct Theme {
    // Borders & focus
    /// Focused/active accent color. Used for borders, selected gutter, active tabs.
    pub focus_accent: Color,
    /// Unfocused/inactive border color.
    pub border_unfocused: Color,

    // Text
    /// Primary content text color (assistant messages, user messages, values).
    pub primary_text: Color,
    /// Muted/dim text (system entries, descriptions, table separators).
    pub muted_text: Color,
    /// Error text color.
    pub error_text: Color,

    // Status
    /// Success/healthy status color (running actors, fresh data, active markers).
    pub success: Color,
    /// Warning/starting status color (stale data, starting actors).
    pub warning: Color,
    /// Streaming indicator color.
    pub streaming: Color,

    // Chat log backgrounds
    /// Gutter column background (left margin).
    pub gutter_bg: Color,
    /// User message block background.
    pub user_block_bg: Color,
    /// Tool call/result text foreground.
    pub tool_block_fg: Color,
    /// Tool result success block background.
    pub tool_success_bg: Color,
    /// Tool result failure block background.
    pub tool_failure_bg: Color,
    /// Tool result truncation indicator foreground.
    pub truncation_fg: Color,

    // Picker
    /// Picker active item marker color (the `>` prefix).
    pub picker_active_marker: Color,
    /// Picker selected row background.
    pub picker_selected_bg: Color,
    /// Picker fuzzy match highlight background.
    pub picker_highlight_bg: Color,

    // Tab bar
    /// Active tab text color.
    pub tab_active_fg: Color,
    /// Active tab background color.
    pub tab_active_bg: Color,
    /// Inactive tab text color.
    pub tab_inactive_fg: Color,

    // Selection highlight
    /// Selection highlight foreground (fallback for identical fg/bg).
    pub selection_fg: Color,
    /// Selection highlight background (fallback for identical fg/bg).
    pub selection_bg: Color,

    // Provider picker
    /// Accent action color (e.g., "CTRL+R to refresh").
    pub accent_action: Color,
    /// Fresh data age color.
    pub age_fresh: Color,
    /// Stale data age color.
    pub age_stale: Color,

    // Scroll indicator
    /// Scroll indicator background.
    pub scroll_indicator_bg: Color,
}

/// TOML-serializable theme file with optional fields.
///
/// All fields are `Option<ThemeColor>`. Missing fields are resolved from
/// the default theme when loading.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ThemeFile {
    #[serde(default)]
    pub focus_accent: Option<ThemeColor>,
    #[serde(default)]
    pub border_unfocused: Option<ThemeColor>,

    #[serde(default)]
    pub primary_text: Option<ThemeColor>,
    #[serde(default)]
    pub muted_text: Option<ThemeColor>,
    #[serde(default)]
    pub error_text: Option<ThemeColor>,

    #[serde(default)]
    pub success: Option<ThemeColor>,
    #[serde(default)]
    pub warning: Option<ThemeColor>,
    #[serde(default)]
    pub streaming: Option<ThemeColor>,

    #[serde(default)]
    pub gutter_bg: Option<ThemeColor>,
    #[serde(default)]
    pub user_block_bg: Option<ThemeColor>,
    #[serde(default)]
    pub tool_block_fg: Option<ThemeColor>,
    #[serde(default)]
    pub tool_success_bg: Option<ThemeColor>,
    #[serde(default)]
    pub tool_failure_bg: Option<ThemeColor>,
    #[serde(default)]
    pub truncation_fg: Option<ThemeColor>,

    #[serde(default)]
    pub picker_active_marker: Option<ThemeColor>,
    #[serde(default)]
    pub picker_selected_bg: Option<ThemeColor>,
    #[serde(default)]
    pub picker_highlight_bg: Option<ThemeColor>,

    #[serde(default)]
    pub tab_active_fg: Option<ThemeColor>,
    #[serde(default)]
    pub tab_active_bg: Option<ThemeColor>,
    #[serde(default)]
    pub tab_inactive_fg: Option<ThemeColor>,

    #[serde(default)]
    pub selection_fg: Option<ThemeColor>,
    #[serde(default)]
    pub selection_bg: Option<ThemeColor>,

    #[serde(default)]
    pub accent_action: Option<ThemeColor>,
    #[serde(default)]
    pub age_fresh: Option<ThemeColor>,
    #[serde(default)]
    pub age_stale: Option<ThemeColor>,

    #[serde(default)]
    pub scroll_indicator_bg: Option<ThemeColor>,
}

impl ThemeFile {
    /// Resolves this file into a full [`Theme`], filling missing fields
    /// from the default theme.
    #[must_use]
    pub fn resolve(&self) -> Theme {
        let d = default_theme();
        Theme {
            focus_accent: self.focus_accent.map(|c| c.inner()).unwrap_or(d.focus_accent),
            border_unfocused: self
                .border_unfocused
                .map(|c| c.inner())
                .unwrap_or(d.border_unfocused),
            primary_text: self.primary_text.map(|c| c.inner()).unwrap_or(d.primary_text),
            muted_text: self.muted_text.map(|c| c.inner()).unwrap_or(d.muted_text),
            error_text: self.error_text.map(|c| c.inner()).unwrap_or(d.error_text),
            success: self.success.map(|c| c.inner()).unwrap_or(d.success),
            warning: self.warning.map(|c| c.inner()).unwrap_or(d.warning),
            streaming: self.streaming.map(|c| c.inner()).unwrap_or(d.streaming),
            gutter_bg: self.gutter_bg.map(|c| c.inner()).unwrap_or(d.gutter_bg),
            user_block_bg: self
                .user_block_bg
                .map(|c| c.inner())
                .unwrap_or(d.user_block_bg),
            tool_block_fg: self
                .tool_block_fg
                .map(|c| c.inner())
                .unwrap_or(d.tool_block_fg),
            tool_success_bg: self
                .tool_success_bg
                .map(|c| c.inner())
                .unwrap_or(d.tool_success_bg),
            tool_failure_bg: self
                .tool_failure_bg
                .map(|c| c.inner())
                .unwrap_or(d.tool_failure_bg),
            truncation_fg: self
                .truncation_fg
                .map(|c| c.inner())
                .unwrap_or(d.truncation_fg),
            picker_active_marker: self
                .picker_active_marker
                .map(|c| c.inner())
                .unwrap_or(d.picker_active_marker),
            picker_selected_bg: self
                .picker_selected_bg
                .map(|c| c.inner())
                .unwrap_or(d.picker_selected_bg),
            picker_highlight_bg: self
                .picker_highlight_bg
                .map(|c| c.inner())
                .unwrap_or(d.picker_highlight_bg),
            tab_active_fg: self
                .tab_active_fg
                .map(|c| c.inner())
                .unwrap_or(d.tab_active_fg),
            tab_active_bg: self
                .tab_active_bg
                .map(|c| c.inner())
                .unwrap_or(d.tab_active_bg),
            tab_inactive_fg: self
                .tab_inactive_fg
                .map(|c| c.inner())
                .unwrap_or(d.tab_inactive_fg),
            selection_fg: self.selection_fg.map(|c| c.inner()).unwrap_or(d.selection_fg),
            selection_bg: self.selection_bg.map(|c| c.inner()).unwrap_or(d.selection_bg),
            accent_action: self
                .accent_action
                .map(|c| c.inner())
                .unwrap_or(d.accent_action),
            age_fresh: self.age_fresh.map(|c| c.inner()).unwrap_or(d.age_fresh),
            age_stale: self.age_stale.map(|c| c.inner()).unwrap_or(d.age_stale),
            scroll_indicator_bg: self
                .scroll_indicator_bg
                .map(|c| c.inner())
                .unwrap_or(d.scroll_indicator_bg),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[rstest::rstest]
    fn empty_theme_file_resolves_to_default() {
        // Given an empty theme file (all None).
        let file = ThemeFile {
            focus_accent: None,
            border_unfocused: None,
            primary_text: None,
            muted_text: None,
            error_text: None,
            success: None,
            warning: None,
            streaming: None,
            gutter_bg: None,
            user_block_bg: None,
            tool_block_fg: None,
            tool_success_bg: None,
            tool_failure_bg: None,
            truncation_fg: None,
            picker_active_marker: None,
            picker_selected_bg: None,
            picker_highlight_bg: None,
            tab_active_fg: None,
            tab_active_bg: None,
            tab_inactive_fg: None,
            selection_fg: None,
            selection_bg: None,
            accent_action: None,
            age_fresh: None,
            age_stale: None,
            scroll_indicator_bg: None,
        };

        // When resolving.
        let theme = file.resolve();
        let default = default_theme();

        // Then all fields match the default theme.
        assert_eq!(theme.focus_accent, default.focus_accent);
        assert_eq!(theme.muted_text, default.muted_text);
        assert_eq!(theme.gutter_bg, default.gutter_bg);
    }

    #[rstest::rstest]
    fn partial_theme_file_overrides_only_specified() {
        // Given a theme file with only focus_accent set.
        let file = ThemeFile {
            focus_accent: Some(ThemeColor(ratatui::style::Color::Red)),
            border_unfocused: None,
            primary_text: None,
            muted_text: None,
            error_text: None,
            success: None,
            warning: None,
            streaming: None,
            gutter_bg: None,
            user_block_bg: None,
            tool_block_fg: None,
            tool_success_bg: None,
            tool_failure_bg: None,
            truncation_fg: None,
            picker_active_marker: None,
            picker_selected_bg: None,
            picker_highlight_bg: None,
            tab_active_fg: None,
            tab_active_bg: None,
            tab_inactive_fg: None,
            selection_fg: None,
            selection_bg: None,
            accent_action: None,
            age_fresh: None,
            age_stale: None,
            scroll_indicator_bg: None,
        };

        // When resolving.
        let theme = file.resolve();
        let default = default_theme();

        // Then focus_accent is overridden.
        assert_eq!(theme.focus_accent, Color::Red);
        // And other fields remain default.
        assert_eq!(theme.muted_text, default.muted_text);
        assert_eq!(theme.gutter_bg, default.gutter_bg);
    }

    #[rstest::rstest]
    fn theme_file_from_toml() {
        // Given a TOML string with one field.
        let toml_str = "focus_accent = \"red\"";
        let file: ThemeFile = toml::from_str(toml_str).expect("parse");

        // When resolving.
        let theme = file.resolve();

        // Then focus_accent is Red and everything else is default.
        assert_eq!(theme.focus_accent, Color::Red);
        assert_eq!(
            theme.muted_text,
            default_theme().muted_text
        );
    }

    #[rstest::rstest]
    fn theme_file_round_trip() {
        // Given a theme file with all fields set.
        let original = ThemeFile {
            focus_accent: Some(ThemeColor(Color::Rgb(255, 0, 0))),
            border_unfocused: Some(ThemeColor(Color::DarkGray)),
            primary_text: Some(ThemeColor(Color::White)),
            muted_text: Some(ThemeColor(Color::DarkGray)),
            error_text: Some(ThemeColor(Color::Red)),
            success: Some(ThemeColor(Color::Green)),
            warning: Some(ThemeColor(Color::Yellow)),
            streaming: Some(ThemeColor(Color::Cyan)),
            gutter_bg: Some(ThemeColor(Color::Rgb(25, 27, 30))),
            user_block_bg: Some(ThemeColor(Color::Rgb(52, 53, 65))),
            tool_block_fg: Some(ThemeColor(Color::Rgb(88, 95, 106))),
            tool_success_bg: Some(ThemeColor(Color::Rgb(40, 50, 40))),
            tool_failure_bg: Some(ThemeColor(Color::Rgb(60, 40, 40))),
            truncation_fg: Some(ThemeColor(Color::Rgb(83, 83, 83))),
            picker_active_marker: Some(ThemeColor(Color::Green)),
            picker_selected_bg: Some(ThemeColor(Color::DarkGray)),
            picker_highlight_bg: Some(ThemeColor(Color::DarkGray)),
            tab_active_fg: Some(ThemeColor(Color::Black)),
            tab_active_bg: Some(ThemeColor(Color::Yellow)),
            tab_inactive_fg: Some(ThemeColor(Color::Gray)),
            selection_fg: Some(ThemeColor(Color::Black)),
            selection_bg: Some(ThemeColor(Color::White)),
            accent_action: Some(ThemeColor(Color::Rgb(255, 165, 0))),
            age_fresh: Some(ThemeColor(Color::LightGreen)),
            age_stale: Some(ThemeColor(Color::Red)),
            scroll_indicator_bg: Some(ThemeColor(Color::Black)),
        };

        // When serializing to TOML and back.
        let toml_str = toml::to_string(&original).expect("serialize");
        let restored: ThemeFile = toml::from_str(&toml_str).expect("parse");

        // Then the resolved themes are identical.
        assert_eq!(
            original.resolve().focus_accent,
            restored.resolve().focus_accent
        );
        assert_eq!(
            original.resolve().gutter_bg,
            restored.resolve().gutter_bg
        );
    }
}
