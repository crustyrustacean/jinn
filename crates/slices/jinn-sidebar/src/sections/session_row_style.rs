//! Shared selection idiom for every sidebar section.
//!
//! A selected row is a full-width band, not a terminal inversion: the line
//! carries `bg(selection_fg)` — the theme's *bright* selection color — and a
//! trailing band-styled pad extends it past the last grapheme, because
//! `Paragraph` never extends a line's style into the cells after its content.
//! The band's text is [`gutter_bg`](jinn_theme::Theme::gutter_bg), the
//! sidebar's own background, so a selected row is a literal inversion of the
//! panel it sits on. State colors are untouched by selection: an attendant
//! stays pink until it is selected, and selection is the only thing that
//! overrides it.
//!
//! [`gutter_span`] is the fixed-width gutter cell every section's content
//! rows are indented by — one dark column between the row's edge and the
//! content, on selected and unselected rows alike.

use jinn_theme::Theme;
use ratatui::style::Style;
use ratatui::text::Span;

/// The style every selected sidebar row reduces to.
///
/// Bright selection background with dark text — the sidebar's own background —
/// so a selected row is a literal inversion of the panel it sits on.
#[must_use]
pub fn selected_row_style(theme: &Theme) -> Style {
    Style::default().fg(theme.gutter_bg).bg(theme.selection_fg)
}

/// Whether the row the cursor is on renders in this exact style.
#[must_use]
pub fn is_selected_row_style(style: Style, theme: &Theme) -> bool {
    style.fg == Some(theme.gutter_bg)
        && style.bg == Some(theme.selection_fg)
        && style.add_modifier.is_empty()
}

/// The single dark gutter cell ahead of a section's content rows.
///
/// The sidebar's content is indented one column from its edge on every row,
/// selected or not; the dark background keeps the column visible against the
/// selection band.
#[must_use]
pub fn gutter_span(theme: &Theme) -> Span<'static> {
    Span::styled(" ", Style::default().bg(theme.gutter_bg))
}

/// Extends a selected row's band to the row's full width.
///
/// `Paragraph` paints only its own style across the area and then draws each
/// grapheme with its own style — it never extends a line's style into the
/// cells past the last grapheme. A selected row therefore carries an explicit
/// band-styled pad from its content's end to `row_width`, so the band reaches
/// the row's last cell.
#[must_use]
pub fn band_pad(content_width: usize, row_width: usize, theme: &Theme) -> Span<'static> {
    Span::styled(
        " ".repeat(row_width.saturating_sub(content_width)),
        selected_row_style(theme),
    )
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use super::*;

    #[rstest::rstest]
    #[test]
    fn selected_row_style_pairs_dark_text_on_bright_selection_background() {
        // Given the default theme.
        let theme = jinn_theme::default_theme();

        // When the selection style is built.
        let style = selected_row_style(&theme);

        // Then it is the sidebar background as text on the bright selection
        // background — a literal inversion of the panel.
        assert_eq!(style.fg, Some(theme.gutter_bg));
        assert_eq!(style.bg, Some(theme.selection_fg));
    }

    #[rstest::rstest]
    #[test]
    fn the_selection_band_carries_no_modifiers() {
        // Given the default theme.
        let theme = jinn_theme::default_theme();

        // When the selection style is built.
        let style = selected_row_style(&theme);

        // Then it carries no terminal inversion or other modifier — selection
        // is a color decision, not a terminal capability.
        assert!(style.add_modifier.is_empty());
        assert!(style.sub_modifier.is_empty());
    }

    #[rstest::rstest]
    #[test]
    fn the_gutter_is_a_dark_blank() {
        // Given the default theme.
        let theme = jinn_theme::default_theme();

        // When the gutter span is built.
        let gutter = gutter_span(&theme);

        // Then it is a single blank on the dark sidebar background — one
        // column of separation that stays dark even on a selected row.
        assert_eq!(gutter.content, " ");
        assert_eq!(gutter.style.bg, Some(theme.gutter_bg));
    }
}
