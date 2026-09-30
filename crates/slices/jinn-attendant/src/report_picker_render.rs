//! The attendant report-history picker's overlay rendering and geometry.
//!
//! A slice-owned picker renders from the slice's own cell, mirroring the
//! theme picker: the render context never sees `AppState`, so the opener
//! snapshots the reports into the cell and the render pass reads only that.

use jinn_attendant_msg::AttendantReportPickerState;
use jinn_slices::RenderFacts;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;

/// The popup rect for the report picker: the shared selection-widget popup.
#[must_use]
pub fn report_picker_overlay_rect(area: &Rect) -> Option<Rect> {
    Some(jinn_selection_widget::compute_popup_rect(*area))
}

/// The report picker's palette: chrome unthemed, accents from the active
/// theme, the attendant token on the run header.
#[must_use]
pub fn report_picker_palette(theme: &jinn_theme::Theme) -> jinn_picker::Palette {
    jinn_picker::Palette {
        border: ratatui::style::Color::DarkGray,
        filter_text: ratatui::style::Color::White,
        separator: ratatui::style::Color::DarkGray,
        footer: ratatui::style::Color::DarkGray,
        highlight_bg: ratatui::style::Color::DarkGray,
        muted_text: theme.muted_text,
        accent_action: theme.attendant_fg,
        popup_title: theme.popup_title,
        primary_text: theme.primary_text,
    }
}

/// The picker's declared binds, in footer order.
///
/// Sourced from the rows the slice actually attaches; the footer can never
/// advertise a key the picker does not bind.
#[must_use]
pub fn report_picker_binds() -> Vec<jinn_picker::BindRow> {
    crate::report_picker_routes::REPORT_PICKER_BINDINGS
        .iter()
        .map(|(notation, label)| jinn_picker::BindRow {
            notation,
            label,
            category_hint: "input",
        })
        .collect()
}

/// Draws the report-history picker for one frame.
pub fn render_report_picker(frame: &mut Frame<'_>, area: Rect, facts: &RenderFacts) {
    let Some(cell) = facts
        .slices
        .reader(&jinn_attendant_msg::attendant_report_picker_slot())
    else {
        return;
    };

    // Measure the popup's result rows and write them back for the
    // navigation keys, which need a real row count to keep the highlight
    // on screen.
    let measured = crate::report_picker_viewport::results_viewport(area);
    cell.update(|state: &mut AttendantReportPickerState| state.results_viewport = measured);

    let guard = cell.read();
    let state: &AttendantReportPickerState = &guard;

    let theme = &facts.theme;
    let palette = report_picker_palette(theme);
    let binds = report_picker_binds();
    let keybind = jinn_picker::keybind_line(&binds, jinn_picker::Tail::Standard, &palette);
    let footers = vec![report_status(state, theme), Line::from(keybind.0)];

    let widget = jinn_selection_widget::SelectionWidget::new(&state.selection)
        .title(Line::from(" Reports "))
        .title_style(Style::default().fg(theme.popup_title))
        .footers(footers)
        .colors(palette.selection_colors());

    widget.render(frame, area);
}

/// The picker's status line: how many reports the attendant has published,
/// and which one the highlight is on.
fn report_status(state: &AttendantReportPickerState, theme: &jinn_theme::Theme) -> Line<'static> {
    let total = state.selection.filtered_count();
    if total == 0 {
        return Line::from(jinn_attendant_msg::AttendantReport::EMPTY_MARKER.to_owned())
            .style(Style::default().fg(theme.muted_text));
    }
    let index = state.selection.selection().saturating_add(1);
    Line::from(format!("report {index} of {total} (newest first)"))
        .style(Style::default().fg(theme.muted_text))
}
