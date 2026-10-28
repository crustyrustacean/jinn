//! The saved-bots picker's overlay rendering and geometry.
//!
//! A slice-owned picker renders from the slice's own cell, mirroring the
//! report-history picker: the render context never sees `AppState` or the
//! configuration layer, so the opener snapshots the entries in and the
//! render pass reads only the cell.

use jinn_bot_msg::BotSavedPickerState;
use jinn_slices::RenderFacts;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;

/// The popup rect for the saved-bots picker: the shared
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
        // the key glyphs are keys, not bot content.
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

/// Draws the saved-bots picker for one frame.
pub fn render_saved_picker(frame: &mut Frame<'_>, area: Rect, facts: &RenderFacts) {
    let Some(cell) = facts.slices.reader(&jinn_bot_msg::bot_saved_picker_slot()) else {
        return;
    };

    // The guard must end before the write-back below: `TypedCell` wraps a
    // non-reentrant lock, so writing while the read guard is held would
    // deadlock the render thread.
    let layout = {
        let guard = cell.read();
        let state: &BotSavedPickerState = &guard;
        saved_picker_widget(state, &facts.theme).render(frame, area)
    };

    // Write back what the widget actually solved, for the navigation keys,
    // which need a real row count to keep the highlight on screen.
    cell.update(|state: &mut BotSavedPickerState| {
        state.results_viewport = usize::from(layout.inner.height);
    });
}

/// Assembles the picker's widget: title, status and keybind footers, and
/// palette, from the cell's state and the live theme.
///
/// Both the render pass and the render tests build through this helper, so
/// a test's second construction is identical to what the frame drew.
pub(crate) fn saved_picker_widget<'a>(
    state: &'a BotSavedPickerState,
    theme: &jinn_theme::Theme,
) -> jinn_selection_widget::SelectionWidget<
    'a,
    jinn_picker::PickerEntry<jinn_bot_msg::SavedBotSummary>,
> {
    let palette = saved_picker_palette(theme);
    let binds = saved_picker_binds();
    let keybind = jinn_picker::keybind_line(&binds, jinn_picker::Tail::Standard, &palette);
    let footers = vec![saved_status(state, theme), Line::from(keybind.0)];
    jinn_selection_widget::SelectionWidget::new(&state.selection)
        .title(Line::from(" Saved Bots "))
        .title_style(Style::default().fg(theme.popup_title))
        .footers(footers)
        .colors(palette.selection_colors())
}

/// The picker's status line: how many bots the document holds.
///
/// The total is deliberately the unfiltered count — how many bots
/// exist is a property of `jinn.toml`, not of the half-typed filter, and a
/// count that shrinks as you type reads as bots being deleted.
/// Where the highlight sits is already the list widget's own business.
pub(crate) fn saved_status(
    state: &BotSavedPickerState,
    theme: &jinn_theme::Theme,
) -> Line<'static> {
    let total = state.selection.items().len();
    let noun = if total == 1 { "bot" } else { "bots" };
    Line::from(format!("{total} saved {noun}")).style(Style::default().fg(theme.muted_text))
}
