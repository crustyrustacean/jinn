//! The stream-rule picker's overlay rendering.
//!
//! A slice-owned picker renders from the slice's own cell. The render context
//! deliberately never sees `AppState` (see `RenderFacts`), so this reads
//! `StreamRulePickerState` straight off the registered slot.

use jinn_slices::RenderFacts;
use jinn_stream_rule_picker_msg::StreamRulePickerState;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;

/// The popup rect for the stream-rule picker: the shared selection-widget
/// popup.
#[must_use]
pub fn stream_rule_picker_overlay_rect(area: &Rect) -> Option<Rect> {
    Some(jinn_selection_widget::compute_popup_rect(*area))
}

/// The colors the stream-rule picker draws with — the same chrome/accent
/// split the sibling pickers use.
#[must_use]
pub fn stream_rule_picker_palette(theme: &jinn_theme::Theme) -> jinn_picker::Palette {
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

/// The stream-rule picker's declared binds, in footer order.
///
/// Sourced from the route rows the slice actually attaches, so the footer
/// can never advertise a key the picker does not bind.
#[must_use]
pub fn stream_rule_picker_binds() -> Vec<jinn_picker::BindRow> {
    crate::stream_rule_picker_routes::STREAM_RULE_PICKER_BINDINGS
        .iter()
        .map(|(notation, label)| jinn_picker::BindRow {
            notation,
            label,
            category_hint: "input",
        })
        .collect()
}

/// Draws the stream-rule picker popup for one frame.
pub fn render_stream_rule_picker(frame: &mut Frame<'_>, area: Rect, facts: &RenderFacts) {
    let Some(cell) = facts
        .slices
        .reader(&jinn_stream_rule_picker_msg::stream_rule_picker_slot())
    else {
        return;
    };

    // Measure the popup's result rows and publish them for the navigation
    // keys, which need a real row count to keep the highlight on screen.
    let measured = crate::stream_rule_picker_viewport::results_viewport(area);
    cell.update(|state: &mut StreamRulePickerState| state.results_viewport = measured);

    let guard = cell.read();
    let state: &StreamRulePickerState = &guard;

    let theme = &facts.theme;
    let palette = stream_rule_picker_palette(theme);
    let binds = stream_rule_picker_binds();
    let keybind = jinn_picker::keybind_line(&binds, jinn_picker::Tail::Standard, &palette);

    // The status line (confirm feedback, or a failure's reason) rides above
    // the keybind line — the same two-row footer shape the preview pickers
    // use for their preview hints. Absent status, a blank line keeps the
    // geometry stable so a confirm does not resize the popup.
    let status_line = state.status.clone().unwrap_or_default();
    let footers = vec![Line::from(status_line), Line::from(keybind.0)];

    let widget = jinn_selection_widget::SelectionWidget::new(&state.selection)
        .title(Line::from(" Stream Rules "))
        .title_style(Style::default().fg(theme.popup_title))
        .footers(footers)
        .colors(palette.selection_colors());

    widget.render(frame, area);
}
