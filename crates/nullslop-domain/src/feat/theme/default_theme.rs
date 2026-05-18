//! Default theme — parsed from the embedded `themes/default.toml` at compile time.

use super::theme::{Theme, ThemeFile};

/// Embedded default theme TOML — the single source of truth.
const DEFAULT_TOML: &str = include_str!("../../../../../themes/default.toml");

/// Returns the default theme, parsed from the embedded TOML file.
///
/// The TOML is included at compile time via `include_str!` and parsed on
/// every call. The string is small (~1KB) so this is fast enough without
/// caching. Changing `themes/default.toml` changes the default appearance
/// of the application.
///
/// Uses [`ThemeFile::resolve_standalone`] to avoid calling back into this
/// function (which would be infinite recursion via [`ThemeFile::resolve`]).
#[must_use]
pub fn default_theme() -> Theme {
    let file: ThemeFile = toml::from_str(DEFAULT_TOML)
        .expect("embedded default.toml should be valid — this is a compile-time bug");
    file.resolve_standalone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[rstest::rstest]
    fn default_theme_has_no_reset_colors() {
        // Given the default theme.
        let theme = default_theme();

        // Then no field is Color::Reset (every field should have a concrete value).
        let fields: Vec<(&str, ratatui::style::Color)> = vec![
            ("focus_accent", theme.focus_accent),
            ("border_unfocused", theme.border_unfocused),
            ("primary_text", theme.primary_text),
            ("muted_text", theme.muted_text),
            ("error_text", theme.error_text),
            ("success", theme.success),
            ("warning", theme.warning),
            ("streaming", theme.streaming),
            ("gutter_bg", theme.gutter_bg),
            ("user_block_bg", theme.user_block_bg),
            ("tool_block_fg", theme.tool_block_fg),
            ("tool_success_bg", theme.tool_success_bg),
            ("tool_failure_bg", theme.tool_failure_bg),
            ("tool_pending_bg", theme.tool_pending_bg),
            ("truncation_fg", theme.truncation_fg),
            ("picker_active_marker", theme.picker_active_marker),
            ("picker_selected_bg", theme.picker_selected_bg),
            ("picker_highlight_bg", theme.picker_highlight_bg),
            ("tab_active_fg", theme.tab_active_fg),
            ("tab_active_bg", theme.tab_active_bg),
            ("tab_inactive_fg", theme.tab_inactive_fg),
            ("selection_fg", theme.selection_fg),
            ("selection_bg", theme.selection_bg),
            ("accent_action", theme.accent_action),
            ("age_fresh", theme.age_fresh),
            ("age_stale", theme.age_stale),
            ("scroll_indicator_bg", theme.scroll_indicator_bg),
            ("sidebar_resize_accent", theme.sidebar_resize_accent),
        ];

        for (name, color) in &fields {
            assert_ne!(
                *color,
                ratatui::style::Color::Reset,
                "default theme field '{name}' should not be Reset"
            );
        }
    }

    #[rstest::rstest]
    fn default_theme_parses_embedded_toml() {
        // Given the embedded default.toml.
        // When calling default_theme().
        let theme = default_theme();

        // Then it returns a theme without panicking (proves the TOML parses).
        // Spot-check one field to confirm it's not Reset.
        assert_ne!(theme.focus_accent, ratatui::style::Color::Reset);
    }
}
