//! The saved-attendants picker's state operations.
//!
//! Pure functions over the picker's own cell state. The entries are read
//! from the configuration layer by the opener and snapshotted in here, so
//! the render pass never touches `jinn.toml` — and neither does this
//! module, which keeps the picker inside its slice.

use jinn_attendant_msg::{AttendantSavedPickerState, SavedAttendantSummary};
use jinn_picker::{PickerItemHooks, make_items_with_hooks};
use jinn_preferences_config::schemas::AttendantEntryConfig;
use ratatui::text::{Line, Span};

/// Renders one picker row: the saved attendant's name, in the ordinary
/// text color the widget draws with.
///
/// The name is the whole row. The picker answers "which of the attendants I
/// saved do I want", and the name is how the user knows which is which;
/// the run configuration every row would show is the same shape on all of
/// them, and reading it before choosing is a second thing to do.
#[must_use]
pub fn saved_row(entry: &SavedAttendantSummary, ctx: &jinn_picker::RowCtx<'_>) -> Line<'static> {
    let base = if ctx.is_selected {
        ratatui::style::Style::default().add_modifier(ratatui::style::Modifier::REVERSED)
    } else {
        ratatui::style::Style::default()
    };
    Line::from(Span::styled(entry.name.clone(), base))
}

/// The picker's rows' payload, built from the document's entries.
///
/// The summary is the name and nothing else: the picker creates an
/// attendant, it does not describe one.
#[must_use]
pub fn summaries_of(entries: &[AttendantEntryConfig]) -> Vec<SavedAttendantSummary> {
    entries
        .iter()
        .map(|entry| SavedAttendantSummary {
            name: entry.name.clone(),
        })
        .collect()
}

/// Builds the picker's rows from the live configuration entries.
///
/// One row per saved attendant, in the order the document lists them, which
/// is the order the user or a hand edit put them in.
#[must_use]
pub fn build_rows(
    entries: Vec<SavedAttendantSummary>,
) -> Vec<jinn_picker::PickerEntry<SavedAttendantSummary>> {
    make_items_with_hooks(
        entries,
        PickerItemHooks::new()
            .row(saved_row)
            .search(|entry: &SavedAttendantSummary| entry.name.clone()),
    )
}

/// Opens the picker over a snapshot of the saved attendants.
pub fn open(state: &mut AttendantSavedPickerState, entries: Vec<SavedAttendantSummary>) {
    state.selection.reset();
    state.selection.set_items(build_rows(entries));
}

/// The highlighted entry's name, if a row is highlighted.
#[must_use]
pub fn highlighted_name(state: &AttendantSavedPickerState) -> Option<String> {
    state
        .selection
        .selected_item()
        .map(|item| item.entry().name.clone())
}
