//! The attendant report-history picker's state operations.
//!
//! Pure functions over the picker's own cell state. They read nothing from
//! `AppState` — the reports are snapshotted in by the opener — which keeps
//! the picker inside its slice.

use jinn_attendant_msg::{AttendantReport, AttendantReportPickerState};
use jinn_picker::PickerItemHooks;
use jinn_picker::make_items_with_hooks;
use ratatui::text::Line;

/// Renders one picker row: the run header in the attendant token, then the
/// report's first line. Selected rows invert with the picker's highlight.
#[must_use]
pub fn report_row(report: &AttendantReport, ctx: &jinn_picker::RowCtx<'_>) -> Line<'static> {
    let mut spans = Vec::new();
    let base = if ctx.is_selected {
        ratatui::style::Style::default().add_modifier(ratatui::style::Modifier::REVERSED)
    } else {
        ratatui::style::Style::default()
    };
    spans.push(ratatui::text::Span::styled(
        format!("#{} ", report.run),
        base.fg(ratatui::style::Color::Yellow),
    ));
    let first_line = report.body.lines().next().unwrap_or_default().to_owned();
    spans.push(ratatui::text::Span::styled(first_line, base));
    Line::from(spans)
}

/// Builds the picker's rows from the snapshotted reports.
///
/// Reports are stored oldest-first; a picker reads top-down, so the newest
/// report lands on the first row.
#[must_use]
pub fn build_rows(reports: Vec<AttendantReport>) -> Vec<jinn_picker::PickerEntry<AttendantReport>> {
    let mut newest_first = reports;
    newest_first.reverse();
    make_items_with_hooks(
        newest_first,
        PickerItemHooks::new()
            .row(report_row)
            .search(|report: &AttendantReport| report.body.clone()),
    )
}

/// Opens the picker over a snapshot of one attendant's reports.
pub fn open(state: &mut AttendantReportPickerState, reports: Vec<AttendantReport>) {
    state.selection.reset();
    state.selection.set_items(build_rows(reports));
}

/// The highlighted report, if any.
#[must_use]
pub fn highlighted(state: &AttendantReportPickerState) -> Option<&AttendantReport> {
    state
        .selection
        .selected_item()
        .map(jinn_picker::PickerEntry::entry)
}
