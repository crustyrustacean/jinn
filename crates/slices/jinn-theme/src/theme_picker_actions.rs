// Copyright (C) 2026 Jayson Lennon
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as
// published by the Free Software Foundation, either version 3 of the
// License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! The theme picker's row building and lifecycle operations.
//!
//! Pure functions over the picker's own state. They read what they need from
//! the slice's theme-entries cell, mutate the picker, and return what the
//! caller needs to turn into messages — they never reach session state
//! themselves, which is what keeps the picker inside its slice.

use jinn_picker::make_items_with_hooks;
use jinn_theme::Theme;
use jinn_theme::ThemeEntry;
use jinn_theme_msg::NamedTheme;
use jinn_theme_msg::ThemePickerState;

/// Renders one picker row: the theme's focus-accent swatch followed by the
/// name (selected rows carry the selection background). Filter matches are
/// not highlighted.
#[must_use]
pub fn theme_row(
    entry: &ThemeEntry,
    ctx: &jinn_picker::RowCtx<'_>,
) -> ratatui::text::Line<'static> {
    let style = if ctx.is_selected {
        ratatui::style::Style::default()
            .fg(entry.theme.primary_text)
            .bg(entry.theme.picker_selected_bg)
    } else {
        ratatui::style::Style::default()
    };

    let swatch = ratatui::text::Span::styled(
        "\u{2588} ".to_owned(), // █
        ratatui::style::Style::default().fg(entry.theme.focus_accent),
    );
    let name = ratatui::text::Span::styled(entry.name.clone(), style);
    ratatui::text::Line::from(vec![swatch, name])
}

/// Builds the picker's rows from the theme slice's entries cell.
///
/// The cell's order is already canonical — the built-in "default" pinned
/// first, the rest in case-insensitive name order, assembled once at slice
/// activation — so the rows are the cell's contents verbatim. Opening the
/// picker never touches the filesystem: without the slice's `activate()` the
/// cell is empty and the picker offers nothing, which is the honest
/// reflection of a slice that never ran.
fn build_theme_entries(scanned: &[NamedTheme]) -> Vec<ThemeEntry> {
    scanned
        .iter()
        .map(|named| ThemeEntry {
            name: named.name.clone(),
            theme: named.theme.clone(),
        })
        .collect()
}

/// Wraps entries for the selection widget, wiring the row renderer and the
/// text the filter matches against.
///
/// Without these hooks the widget draws an empty label on every row, so they
/// are part of the picker's definition rather than decoration.
fn wrap_entries(entries: Vec<ThemeEntry>) -> Vec<jinn_picker::PickerEntry<ThemeEntry>> {
    make_items_with_hooks(
        entries,
        jinn_picker::PickerItemHooks::new()
            .row(theme_row)
            .search(|entry: &ThemeEntry| entry.name.clone()),
    )
}

/// Opens the picker: fresh filter and highlight over the slice's themes, a
/// snapshot of the current theme so escape can put it back, and the
/// persisted name the status line reports.
///
/// The snapshot is what makes live preview safe: the highlight's theme is
/// applied the moment it moves, so without a recorded "before" the user could
/// not return to the theme they started from.
pub fn open(
    state: &mut ThemePickerState,
    scanned: &[NamedTheme],
    theme: &Theme,
    persisted: Option<&str>,
) {
    state.selection.reset();
    state.theme = theme.clone();
    state.preview_original = Some(theme.clone());
    state.persisted_name = persisted.map(str::to_owned);
    state
        .selection
        .set_items(wrap_entries(build_theme_entries(scanned)));
    // Put the cursor on the theme already in force, so the menu opens showing
    // where you are rather than at the top of an arbitrary list.
    //
    // This also makes the preview self-consistent: the highlighted theme is
    // what is applied, and confirm persists the highlighted theme, so opening
    // on the current theme means Enter-on-open is a no-op instead of silently
    // persisting row 0's name without ever applying it.
    highlight_current(state);
}

/// Moves the highlight onto the row naming the theme in force.
///
/// Falls back to the first row when no row matches — a theme in force that is
/// not in the list (a removed file, or an unlisted name) must not leave the
/// picker with no selection at all.
fn highlight_current(state: &mut ThemePickerState) {
    let Some(current) = state.persisted_name.clone() else {
        return;
    };
    let Some(index) = state
        .selection
        .items()
        .iter()
        .position(|item| item.entry().name == current)
    else {
        return;
    };
    state.selection.set_selection(index);
}

/// The status line: the persisted theme name — what escape restores to and
/// what survives a restart — or "default" when none is persisted.
#[must_use]
pub fn theme_status(state: &ThemePickerState, theme: &Theme) -> ratatui::text::Line<'static> {
    let current = state.persisted_name.as_deref().unwrap_or("default");
    ratatui::text::Line::from(vec![
        ratatui::text::Span::styled(
            "Current: ".to_owned(),
            ratatui::style::Style::default().fg(theme.muted_text),
        ),
        ratatui::text::Span::styled(
            current.to_owned(),
            ratatui::style::Style::default().fg(theme.primary_text),
        ),
    ])
}

/// The theme the highlight currently selects, if any.
#[must_use]
pub fn highlighted_theme(state: &ThemePickerState) -> Option<&Theme> {
    state
        .selection
        .selected_item()
        .map(|item| &item.entry().theme)
}

/// The name the highlight currently selects, if any.
#[must_use]
pub fn highlighted_name(state: &ThemePickerState) -> Option<String> {
    state
        .selection
        .selected_item()
        .map(|item| item.entry().name.clone())
}

/// Commits the choice: clears the snapshot so nothing reverts the theme the
/// user just settled on, and names it for persistence.
pub fn confirm(state: &mut ThemePickerState) -> Option<String> {
    let name = highlighted_name(state)?;
    state.preview_original = None;
    Some(name)
}

/// Cancels: hands back the pre-open theme so the caller can restore it, and
/// consumes the snapshot so a second close is a no-op.
pub fn cancel(state: &mut ThemePickerState) -> Option<Theme> {
    state.preview_original.take()
}
