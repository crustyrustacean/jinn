//! Default theme — matches all current hardcoded colors in the codebase.

use ratatui::style::Color;

use super::theme::Theme;

/// Returns the default theme, matching all hardcoded colors currently in use.
///
/// Every color value here corresponds to a `Color::Xxx` that was previously
/// hardcoded in the render pipeline. Changing these values changes the default
/// appearance of the application.
#[must_use]
pub fn default_theme() -> Theme {
    Theme {
        // Borders & focus
        focus_accent: Color::Yellow,
        border_unfocused: Color::DarkGray,

        // Text
        primary_text: Color::White,
        muted_text: Color::DarkGray,
        error_text: Color::Red,

        // Status
        success: Color::Green,
        warning: Color::Yellow,
        streaming: Color::Cyan,

        // Chat log backgrounds
        gutter_bg: Color::Rgb(0x19, 0x1B, 0x1E),
        user_block_bg: Color::Rgb(0x34, 0x35, 0x41),
        tool_block_fg: Color::Rgb(0x58, 0x5F, 0x6A),
        tool_success_bg: Color::Rgb(0x28, 0x32, 0x28),
        tool_failure_bg: Color::Rgb(0x3C, 0x28, 0x28),
        truncation_fg: Color::Rgb(0x53, 0x53, 0x53),

        // Picker
        picker_active_marker: Color::Green,
        picker_selected_bg: Color::DarkGray,
        picker_highlight_bg: Color::DarkGray,

        // Tab bar
        tab_active_fg: Color::Black,
        tab_active_bg: Color::Yellow,
        tab_inactive_fg: Color::Gray,

        // Selection highlight
        selection_fg: Color::Black,
        selection_bg: Color::White,

        // Provider picker
        accent_action: Color::Rgb(255, 165, 0),
        age_fresh: Color::LightGreen,
        age_stale: Color::Red,

        // Scroll indicator
        scroll_indicator_bg: Color::Black,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[rstest::rstest]
    fn default_theme_has_no_reset_colors() {
        // Given the default theme.
        let theme = default_theme();

        // Then no field is Color::Reset (every field should have a concrete value).
        let fields: Vec<(&str, Color)> = vec![
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
        ];

        for (name, color) in &fields {
            assert_ne!(
                *color,
                Color::Reset,
                "default theme field '{name}' should not be Reset"
            );
        }
    }

    #[rstest::rstest]
    fn default_theme_round_trips_through_toml() {
        // Given the default theme.
        let theme = default_theme();

        // When converting to a ThemeFile, serializing, and re-parsing.
        use crate::feat::theme::color::ThemeColor;
        use crate::feat::theme::theme::ThemeFile;

        let file = ThemeFile {
            focus_accent: Some(ThemeColor(theme.focus_accent)),
            border_unfocused: Some(ThemeColor(theme.border_unfocused)),
            primary_text: Some(ThemeColor(theme.primary_text)),
            muted_text: Some(ThemeColor(theme.muted_text)),
            error_text: Some(ThemeColor(theme.error_text)),
            success: Some(ThemeColor(theme.success)),
            warning: Some(ThemeColor(theme.warning)),
            streaming: Some(ThemeColor(theme.streaming)),
            gutter_bg: Some(ThemeColor(theme.gutter_bg)),
            user_block_bg: Some(ThemeColor(theme.user_block_bg)),
            tool_block_fg: Some(ThemeColor(theme.tool_block_fg)),
            tool_success_bg: Some(ThemeColor(theme.tool_success_bg)),
            tool_failure_bg: Some(ThemeColor(theme.tool_failure_bg)),
            truncation_fg: Some(ThemeColor(theme.truncation_fg)),
            picker_active_marker: Some(ThemeColor(theme.picker_active_marker)),
            picker_selected_bg: Some(ThemeColor(theme.picker_selected_bg)),
            picker_highlight_bg: Some(ThemeColor(theme.picker_highlight_bg)),
            tab_active_fg: Some(ThemeColor(theme.tab_active_fg)),
            tab_active_bg: Some(ThemeColor(theme.tab_active_bg)),
            tab_inactive_fg: Some(ThemeColor(theme.tab_inactive_fg)),
            selection_fg: Some(ThemeColor(theme.selection_fg)),
            selection_bg: Some(ThemeColor(theme.selection_bg)),
            accent_action: Some(ThemeColor(theme.accent_action)),
            age_fresh: Some(ThemeColor(theme.age_fresh)),
            age_stale: Some(ThemeColor(theme.age_stale)),
            scroll_indicator_bg: Some(ThemeColor(theme.scroll_indicator_bg)),
        };

        let toml_str = toml::to_string_pretty(&file).expect("serialize");
        let restored: ThemeFile = toml::from_str(&toml_str).expect("parse");
        let resolved = restored.resolve();

        // Then every field matches the original default theme.
        assert_eq!(resolved.focus_accent, theme.focus_accent);
        assert_eq!(resolved.border_unfocused, theme.border_unfocused);
        assert_eq!(resolved.primary_text, theme.primary_text);
        assert_eq!(resolved.muted_text, theme.muted_text);
        assert_eq!(resolved.error_text, theme.error_text);
        assert_eq!(resolved.success, theme.success);
        assert_eq!(resolved.warning, theme.warning);
        assert_eq!(resolved.streaming, theme.streaming);
        assert_eq!(resolved.gutter_bg, theme.gutter_bg);
        assert_eq!(resolved.user_block_bg, theme.user_block_bg);
        assert_eq!(resolved.tool_block_fg, theme.tool_block_fg);
        assert_eq!(resolved.tool_success_bg, theme.tool_success_bg);
        assert_eq!(resolved.tool_failure_bg, theme.tool_failure_bg);
        assert_eq!(resolved.truncation_fg, theme.truncation_fg);
        assert_eq!(resolved.picker_active_marker, theme.picker_active_marker);
        assert_eq!(resolved.picker_selected_bg, theme.picker_selected_bg);
        assert_eq!(resolved.picker_highlight_bg, theme.picker_highlight_bg);
        assert_eq!(resolved.tab_active_fg, theme.tab_active_fg);
        assert_eq!(resolved.tab_active_bg, theme.tab_active_bg);
        assert_eq!(resolved.tab_inactive_fg, theme.tab_inactive_fg);
        assert_eq!(resolved.selection_fg, theme.selection_fg);
        assert_eq!(resolved.selection_bg, theme.selection_bg);
        assert_eq!(resolved.accent_action, theme.accent_action);
        assert_eq!(resolved.age_fresh, theme.age_fresh);
        assert_eq!(resolved.age_stale, theme.age_stale);
        assert_eq!(resolved.scroll_indicator_bg, theme.scroll_indicator_bg);
    }
}
