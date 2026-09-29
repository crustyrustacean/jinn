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
//! Sections that lead with a chip (`█`) get their chip from
//! [`chip_span`], whose explicit dark background keeps the glyph crisp
//! against the band; [`chip_gap`] is the unstyled cell between chip and
//! content that the band visibly stops short of.

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

/// The section's leading chip cell.
///
/// `█` in the focus accent when the section has focus, the unfocused border
/// color when it does not; a blank when the row is not selected. Both cells
/// carry the dark sidebar background so the chip stays crisp against the
/// selection band and the column reads as a gutter even unselected.
#[must_use]
pub fn chip_span(selected: bool, focused: bool, theme: &Theme) -> Span<'static> {
    // The glyph's color follows the *section's* focus, not the row's
    // selection: an unfocused sidebar's cursor is dimmer on every row.
    let color = if focused {
        theme.focus_accent
    } else {
        theme.border_unfocused
    };
    Span::styled(
        if selected { "█" } else { " " },
        Style::default().fg(color).bg(theme.gutter_bg),
    )
}

/// The unstyled cell between the chip and the row's content.
///
/// Deliberately carries no style: on a selected row it takes the band and
/// shows the band stopping short of the chip, which is what makes the chip
/// read as a chip.
#[must_use]
pub fn chip_gap() -> Span<'static> {
    Span::raw("  ")
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
    fn a_selected_chip_is_the_block_in_the_focus_accent_on_the_dark_background() {
        // Given the default theme and a selected row in a focused section.
        let theme = jinn_theme::default_theme();

        // When the chip span is built.
        let chip = chip_span(true, true, &theme);

        // Then the glyph is the block in the focus accent...
        assert_eq!(chip.content, "█");
        assert_eq!(chip.style.fg, Some(theme.focus_accent));
        // And its background is the dark sidebar background, so the chip
        // stays crisp against the selection band.
        assert_eq!(chip.style.bg, Some(theme.gutter_bg));
    }

    #[rstest::rstest]
    #[test]
    fn a_selected_chip_in_an_unfocused_section_uses_the_unfocused_border_color() {
        // Given the default theme and a selected row in an unfocused sidebar.
        let theme = jinn_theme::default_theme();

        // When the chip span is built.
        let chip = chip_span(true, false, &theme);

        // Then the glyph is the block in the unfocused border color.
        assert_eq!(chip.content, "█");
        assert_eq!(chip.style.fg, Some(theme.border_unfocused));
    }

    #[rstest::rstest]
    #[test]
    fn an_unselected_chip_is_a_dark_blank() {
        // Given the default theme and an unselected row.
        let theme = jinn_theme::default_theme();

        // When the chip span is built.
        let chip = chip_span(false, true, &theme);

        // Then the cell is a blank on the dark sidebar background, keeping
        // the gutter column aligned.
        assert_eq!(chip.content, " ");
        assert_eq!(chip.style.bg, Some(theme.gutter_bg));
    }

    #[rstest::rstest]
    #[test]
    fn the_gap_cell_is_an_unstyled_space() {
        // When the gap span is built.
        let gap = chip_gap();

        // Then it is a plain space with no style, so the selection band
        // paints through it — two cells, keeping a visible gap on both the
        // dark gutter and the bright band.
        assert_eq!(gap.content, "  ");
        assert_eq!(gap.style, Style::default());
    }
}
