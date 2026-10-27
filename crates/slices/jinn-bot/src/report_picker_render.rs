//! The bot report-history picker's overlay rendering and geometry.
//!
//! A slice-owned picker renders from the slice's own cell, mirroring the
//! theme picker: the render context never sees `AppState`, so the opener
//! snapshots the reports into the cell and the render pass reads only that.

use jinn_bot_msg::BotReportPickerState;
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
/// theme, the bot token on the run header.
#[must_use]
pub fn report_picker_palette(theme: &jinn_theme::Theme) -> jinn_picker::Palette {
    jinn_picker::Palette {
        border: ratatui::style::Color::DarkGray,
        filter_text: ratatui::style::Color::White,
        separator: ratatui::style::Color::DarkGray,
        footer: ratatui::style::Color::DarkGray,
        highlight_bg: ratatui::style::Color::DarkGray,
        muted_text: theme.muted_text,
        accent_action: theme.bot_fg,
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
    let Some(cell) = facts.slices.reader(&jinn_bot_msg::bot_report_picker_slot()) else {
        return;
    };

    // The guard must end before the write-back below: `TypedCell` wraps a
    // non-reentrant lock, so writing while the read guard is held would
    // deadlock the render thread.
    let layout = {
        let guard = cell.read();
        let state: &BotReportPickerState = &guard;
        report_picker_widget(state, &facts.theme).render(frame, area)
    };

    // Write back what the widget actually solved, for the navigation keys,
    // which need a real row count to keep the highlight on screen.
    cell.update(|state: &mut BotReportPickerState| {
        state.results_viewport = usize::from(layout.inner.height);
    });
}

/// Assembles the picker's widget: title, status and keybind footers, and
/// palette, from the cell's state and the live theme.
///
/// Both the render pass and the render tests build through this helper, so
/// a test's second construction is identical to what the frame drew.
pub(crate) fn report_picker_widget<'a>(
    state: &'a BotReportPickerState,
    theme: &jinn_theme::Theme,
) -> jinn_selection_widget::SelectionWidget<'a, jinn_picker::PickerEntry<jinn_bot_msg::BotReport>> {
    let palette = report_picker_palette(theme);
    let binds = report_picker_binds();
    let keybind = jinn_picker::keybind_line(&binds, jinn_picker::Tail::Standard, &palette);
    let footers = vec![report_status(state, theme), Line::from(keybind.0)];
    jinn_selection_widget::SelectionWidget::new(&state.selection)
        .title(Line::from(" Reports "))
        .title_style(Style::default().fg(theme.popup_title))
        .footers(footers)
        .colors(palette.selection_colors())
}

/// The picker's status line: how many reports the bot has published,
/// and which one the highlight is on.
fn report_status(state: &BotReportPickerState, theme: &jinn_theme::Theme) -> Line<'static> {
    let total = state.selection.filtered_count();
    if total == 0 {
        return Line::from(jinn_bot_msg::BotReport::EMPTY_MARKER.to_owned())
            .style(Style::default().fg(theme.muted_text));
    }
    let index = state.selection.selection().saturating_add(1);
    Line::from(format!("report {index} of {total} (newest first)"))
        .style(Style::default().fg(theme.muted_text))
}
