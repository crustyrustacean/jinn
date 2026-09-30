//! The saved-attendants picker's overlay rendering and geometry.
//!
//! A slice-owned picker renders from the slice's own cell, mirroring the
//! report-history picker: the render context never sees `AppState` or the
//! configuration layer, so the opener snapshots the entries in and the
//! render pass reads only the cell.

use jinn_attendant_msg::AttendantSavedPickerState;
use jinn_slices::RenderFacts;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;

/// The popup rect for the saved-attendants picker: the shared
/// selection-widget popup.
#[must_use]
pub fn saved_picker_overlay_rect(area: &Rect) -> Option<Rect> {
    Some(jinn_selection_widget::compute_popup_rect(*area))
}

/// The picker's palette: chrome unthemed, accents from the active theme.
#[must_use]
pub fn saved_picker_palette(theme: &jinn_theme::Theme) -> jinn_picker::Palette {
    jinn_picker::Palette {
        border: ratatui::style::Color::DarkGray,
        filter_text: ratatui::style::Color::White,
        separator: ratatui::style::Color::DarkGray,
        footer: ratatui::style::Color::DarkGray,
        highlight_bg: ratatui::style::Color::DarkGray,
        muted_text: theme.muted_text,
        // The hotkey accent, as every other picker's keybind line uses:
        // the key glyphs are keys, not attendant content.
        accent_action: theme.accent_action,
        popup_title: theme.popup_title,
        primary_text: theme.primary_text,
    }
}

/// The picker's declared binds, in footer order.
///
/// Sourced from the rows the slice actually attaches; the footer can never
/// advertise a key the picker does not bind.
#[must_use]
pub fn saved_picker_binds() -> Vec<jinn_picker::BindRow> {
    crate::saved_picker_routes::SAVED_PICKER_BINDINGS
        .iter()
        .map(|(notation, label)| jinn_picker::BindRow {
            notation,
            label,
            category_hint: "input",
        })
        .collect()
}

/// Draws the saved-attendants picker for one frame.
pub fn render_saved_picker(frame: &mut Frame<'_>, area: Rect, facts: &RenderFacts) {
    let Some(cell) = facts
        .slices
        .reader(&jinn_attendant_msg::attendant_saved_picker_slot())
    else {
        return;
    };

    // Measure the popup's result rows and write them back for the
    // navigation keys, which need a real row count to keep the highlight
    // on screen.
    let measured = crate::saved_picker_viewport::results_viewport(area);
    cell.update(|state: &mut AttendantSavedPickerState| state.results_viewport = measured);

    let guard = cell.read();
    let state: &AttendantSavedPickerState = &guard;

    let theme = &facts.theme;
    let palette = saved_picker_palette(theme);
    let binds = saved_picker_binds();
    let keybind = jinn_picker::keybind_line(&binds, jinn_picker::Tail::Standard, &palette);
    let footers = vec![saved_status(state, theme), Line::from(keybind.0)];

    let widget = jinn_selection_widget::SelectionWidget::new(&state.selection)
        .title(Line::from(" Saved Attendants "))
        .title_style(Style::default().fg(theme.popup_title))
        .footers(footers)
        .colors(palette.selection_colors());

    widget.render(frame, area);
}

/// The picker's status line: how many attendants the document holds.
///
/// The total is deliberately the unfiltered count — how many attendants
/// exist is a property of `jinn.toml`, not of the half-typed filter, and a
/// count that shrinks as you type reads as attendants being deleted.
/// Where the highlight sits is already the list widget's own business.
pub(crate) fn saved_status(
    state: &AttendantSavedPickerState,
    theme: &jinn_theme::Theme,
) -> Line<'static> {
    let total = state.selection.items().len();
    let noun = if total == 1 {
        "attendant"
    } else {
        "attendants"
    };
    Line::from(format!("{total} saved {noun}")).style(Style::default().fg(theme.muted_text))
}
