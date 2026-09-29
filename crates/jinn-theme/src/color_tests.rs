#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

//! Public-API tests for [`ThemeColor`] TOML serialization.

use ratatui::style::Color;

use super::ThemeColor;

/// Wrapper struct for testing TOML round-trips of individual ThemeColor values.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct ColorWrapper {
    color: ThemeColor,
}

#[rstest::rstest]
fn toml_ansi_name() {
    // Given a TOML string with an ANSI color name.
    let toml_str = "color = \"yellow\"";

    // When deserializing it.
    let wrapper: ColorWrapper = toml::from_str(toml_str).expect("parse");

    // Then it deserializes to the named color.
    assert_eq!(wrapper.color.0, Color::Yellow);
}

#[rstest::rstest]
fn toml_hex() {
    // Given a TOML string with a hex color.
    let toml_str = "color = \"#FFA500\"";

    // When deserializing it.
    let wrapper: ColorWrapper = toml::from_str(toml_str).expect("parse");

    // Then it deserializes to RGB.
    assert_eq!(wrapper.color.0, Color::Rgb(255, 165, 0));
}

#[rstest::rstest]
fn toml_rgb_array() {
    // Given a TOML array with 3 u8 values.
    let toml_str = "color = [25, 27, 30]";

    // When deserializing it.
    let wrapper: ColorWrapper = toml::from_str(toml_str).expect("parse");

    // Then it deserializes to RGB.
    assert_eq!(wrapper.color.0, Color::Rgb(25, 27, 30));
}

#[rstest::rstest]
fn toml_ansi_code() {
    // Given a TOML string with an ANSI code.
    let toml_str = "color = \"A80\"";

    // When deserializing it.
    let wrapper: ColorWrapper = toml::from_str(toml_str).expect("parse");

    // Then it deserializes to an RGB color (resolved via anstyle-lossy).
    assert!(matches!(wrapper.color.0, Color::Rgb(_, _, _)));
}

#[rstest::rstest]
fn toml_invalid_string_fails() {
    // Given a TOML string that is not a valid color.
    let toml_str = "color = \"notacolor123\"";

    // When deserializing it.
    let result: Result<ColorWrapper, _> = toml::from_str(toml_str);

    // Then deserialization fails.
    assert!(result.is_err());
}

#[rstest::rstest]
fn toml_invalid_array_fails() {
    // Given a TOML array with only 2 values.
    let toml_str = "color = [255, 165]";

    // When deserializing it.
    let result: Result<ColorWrapper, _> = toml::from_str(toml_str);

    // Then deserialization fails.
    assert!(result.is_err());
}

#[rstest::rstest]
fn serialize_rgb_round_trip() {
    // Given a ThemeColor with RGB.
    let original = ColorWrapper {
        color: ThemeColor(Color::Rgb(255, 165, 0)),
    };
    // When serializing to TOML and back.
    let toml_str = toml::to_string(&original).expect("serialize");
    let restored: ColorWrapper = toml::from_str(&toml_str).expect("parse");
    // Then the color is preserved.
    assert_eq!(original.color.0, restored.color.0);
}

#[rstest::rstest]
fn serialize_named_round_trip() {
    // Given a ThemeColor with a named color.
    let original = ColorWrapper {
        color: ThemeColor(Color::Yellow),
    };
    // When serializing to TOML and back.
    let toml_str = toml::to_string(&original).expect("serialize");
    let restored: ColorWrapper = toml::from_str(&toml_str).expect("parse");
    // Then the color is preserved.
    assert_eq!(original.color.0, restored.color.0);
}

#[cfg(test)]
mod bundled_themes_load {
    #![allow(clippy::expect_used, reason = "test assertions")]
    use crate::default_theme;
    use crate::theme::ThemeFile;

    /// Every theme shipped in `res/themes`, with its file contents.
    ///
    /// A new `Theme` field must not break a theme that predates it: the
    /// resolver fills any omitted key from the default, so a bundled file
    /// that simply lacks the new key still has to load. This is the guard
    /// on that, across all of them at once rather than one at a time.
    #[rstest::rstest]
    #[case("default.toml", include_str!("../../../res/themes/default.toml"))]
    #[case("catppuccin-mocha.toml", include_str!("../../../res/themes/catppuccin-mocha.toml"))]
    #[case("gruvbox-dark.toml", include_str!("../../../res/themes/gruvbox-dark.toml"))]
    #[case("nord-light.toml", include_str!("../../../res/themes/nord-light.toml"))]
    #[case("sonokai.toml", include_str!("../../../res/themes/sonokai.toml"))]
    fn every_bundled_theme_parses_and_resolves_every_colour(
        #[case] name: &str,
        #[case] contents: &str,
    ) {
        // Given a bundled theme file.
        let file: ThemeFile = toml::from_str(contents).unwrap_or_else(|e| panic!("{name}: {e}"));

        // When resolving it against the default.
        let theme = file.resolve_with_fallback(&default_theme());

        // Then the new key landed as a real colour rather than a Reset
        // that would silently render as the terminal's own foreground.
        // The four themes that predate the key fall back to the default
        // and the one that sets it uses its own value — both must land
        // on a concrete colour.
        assert_ne!(
            theme.dormant_fg,
            ratatui::style::Color::Reset,
            "{name}: dormant_fg resolved to Reset"
        );
    }

    /// The new token's meaning is "dormant but healthy", so it must not
    /// resolve to the error colour — that is the whole reason it exists
    /// rather than reusing `error_text`.
    #[rstest::rstest]
    #[test]
    fn the_dormant_token_defaults_to_something_other_than_the_error_colour() {
        // Given the default theme.
        let theme = default_theme();

        // Then dormancy is not painted as a failure.
        assert_ne!(
            theme.dormant_fg, theme.error_text,
            "a passivated actor must not read as an error"
        );
    }
}

#[cfg(test)]
mod style_map_integration_tests {
    #![allow(clippy::expect_used, reason = "test assertions")]
    use ratatui::style::Style;

    #[rstest::rstest]
    #[test]
    fn style_map_returns_entry_for_every_theme_field() {
        // Given the default theme.
        let theme = crate::default_theme();
        // When building the style map.
        let map = theme.style_map();
        // Then it has one entry per Theme field.
        assert_eq!(map.len(), 50, "style_map should cover all Theme fields");
    }

    #[rstest::rstest]
    #[test]
    fn style_map_values_are_fg_only_styles() {
        // Given the default theme.
        let theme = crate::default_theme();
        // When building the style map.
        let map = theme.style_map();
        // Then selected entries resolve to Style::default().fg(field).
        assert_eq!(
            map.get("streaming"),
            Some(&Style::default().fg(theme.streaming))
        );
        assert_eq!(
            map.get("accent_action"),
            Some(&Style::default().fg(theme.accent_action))
        );
        assert_eq!(
            map.get("muted_text"),
            Some(&Style::default().fg(theme.muted_text))
        );
        assert_eq!(
            map.get("subagent_fg"),
            Some(&Style::default().fg(theme.subagent_fg))
        );
        assert_eq!(
            map.get("in_flight_bg"),
            Some(&Style::default().fg(theme.in_flight_bg))
        );
        assert_eq!(
            map.get("in_flight_fg"),
            Some(&Style::default().fg(theme.in_flight_fg))
        );
    }
}

#[cfg(test)]
mod in_flight_tint_tests {
    use crate::theme::ThemeFile;

    #[rstest::rstest]
    #[case(include_str!("../../../res/themes/default.toml"), "default")]
    #[case(
        include_str!("../../../res/themes/catppuccin-mocha.toml"),
        "catppuccin-mocha"
    )]
    #[case(
        include_str!("../../../res/themes/gruvbox-dark.toml"),
        "gruvbox-dark"
    )]
    #[case(include_str!("../../../res/themes/nord-light.toml"), "nord-light")]
    #[case(include_str!("../../../res/themes/sonokai.toml"), "sonokai")]
    fn shipped_theme_defines_in_flight_colors(#[case] contents: &str, #[case] name: &str) {
        // Given a bundled theme file.
        let file: ThemeFile = toml::from_str(contents).expect("parse");

        // When reading its in-flight tint colors.
        let bg = file.in_flight_bg;
        let fg = file.in_flight_fg;

        // Then both are authored, so the tint is never invisible.
        assert!(
            bg.is_some(),
            "theme '{name}' must define in_flight_bg for the in-flight tint to render"
        );
        assert!(
            fg.is_some(),
            "theme '{name}' must define in_flight_fg for the in-flight tint to render"
        );
    }

    #[rstest::rstest]
    #[case(include_str!("../../../res/themes/nord-light.toml"))]
    fn light_theme_tint_is_legible_against_its_pale_gutter(#[case] contents: &str) {
        // Given a bundled light theme.
        let file: ThemeFile = toml::from_str(contents).expect("parse");

        // When resolving its in-flight tint and gutter colors.
        let theme = file.resolve();
        let tint_bg = theme.in_flight_bg;
        let gutter = theme.gutter_bg;

        // Then the wash is distinguishable from the gutter behind it.
        assert_ne!(
            tint_bg, gutter,
            "a tint matching the gutter would be invisible on the sidebar"
        );
        // And the text reads against the wash.
        assert_eq!(
            theme.in_flight_fg,
            crate::contrast::ensure_contrast(theme.in_flight_fg, tint_bg),
            "in_flight_fg should already be legible against in_flight_bg"
        );
    }
}
