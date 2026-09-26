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

//! The session-lifecycle picker's overlay rendering.
//!
//! A slice-owned picker renders from the slice's own cell. The render context
//! deliberately never sees `AppState` (see `RenderFacts`), so this reads
//! `SessionLifecyclePickerState` straight off the registered slot — no
//! `PickerHost`, no kernel borrow, and no picker-kind lookup in the kernel.

use jinn_session_lifecycle_msg::SessionLifecyclePickerState;
use jinn_slices::RenderFacts;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;

/// The popup rect for the session-lifecycle picker: the shared
/// selection-widget popup.
#[must_use]
pub fn session_lifecycle_picker_overlay_rect(area: &Rect) -> Option<Rect> {
    Some(jinn_selection_widget::compute_popup_rect(*area))
}

/// The colors the session-lifecycle picker draws with.
///
/// Mirrors the widget's defaults, matching what the kernel-side spec render
/// produced: the chrome fields are unthemed, the accent fields come from the
/// active theme.
#[must_use]
pub fn session_lifecycle_picker_palette(theme: &jinn_theme::Theme) -> jinn_picker::Palette {
    jinn_picker::Palette {
        border: ratatui::style::Color::DarkGray,
        filter_text: ratatui::style::Color::White,
        separator: ratatui::style::Color::DarkGray,
        footer: ratatui::style::Color::DarkGray,
        highlight_bg: ratatui::style::Color::DarkGray,
        muted_text: theme.muted_text,
        accent_action: theme.accent_action,
        popup_title: theme.popup_title,
        primary_text: theme.primary_text,
    }
}

/// The session-lifecycle picker's declared binds, in footer order.
///
/// Sourced from the route rows the slice actually attaches, so the footer can
/// never advertise a key the picker does not bind.
#[must_use]
pub fn session_lifecycle_picker_binds() -> Vec<jinn_picker::BindRow> {
    crate::session_lifecycle_picker_routes::SESSION_LIFECYCLE_PICKER_BINDINGS
        .iter()
        .map(|(notation, label)| jinn_picker::BindRow {
            notation,
            label,
            category_hint: "input",
        })
        .collect()
}

/// Draws the session-lifecycle picker popup for one frame.
pub fn render_session_lifecycle_picker(frame: &mut Frame<'_>, area: Rect, facts: &RenderFacts) {
    let Some(cell) = facts
        .slices
        .reader(&jinn_session_lifecycle_msg::session_lifecycle_picker_slot())
    else {
        return;
    };

    // Measure the popup's result rows and publish them for the navigation keys,
    // which need a real row count to keep the highlight on screen. This is the
    // slice-owned equivalent of the kernel's per-frame viewport write.
    let measured = crate::session_lifecycle_picker_viewport::results_viewport(area);
    cell.update(|state: &mut SessionLifecyclePickerState| state.results_viewport = measured);

    let guard = cell.read();
    let state: &SessionLifecyclePickerState = &guard;

    let theme = &facts.theme;
    let palette = session_lifecycle_picker_palette(theme);
    let binds = session_lifecycle_picker_binds();
    let keybind = jinn_picker::keybind_line(&binds, jinn_picker::Tail::Standard, &palette);

    let widget = jinn_selection_widget::SelectionWidget::new(&state.selection)
        .title(Line::from(" Session Lifecycle "))
        .title_style(Style::default().fg(theme.popup_title))
        .footers(vec![Line::from(keybind.0)])
        .colors(palette.selection_colors());

    widget.render(frame, area);
}
