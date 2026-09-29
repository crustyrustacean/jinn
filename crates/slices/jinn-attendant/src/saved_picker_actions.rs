//! The saved-attendants picker's state operations.
//!
//! Pure functions over the picker's own cell state. The entries are read
//! from the configuration layer by the opener and snapshotted in here, so
//! the render pass never touches `jinn.toml` — and neither does this
//! module, which keeps the picker inside its slice.

use jinn_attendant_msg::{AttendantSavedPickerState, SavedAttendantSummary};
use jinn_picker::{PickerItemHooks, make_items_with_hooks};
use ratatui::style::Color;
use ratatui::text::{Line, Span};

/// Renders one picker row: the entry's name, then its activation and
/// trigger in the muted tone, with the pin count when it has pins.
///
/// The detail is on the row rather than in a preview pane because two saved
/// attendants are almost always told apart by exactly this — a reset
/// reviewer with three pinned instructions and a manual one with none are
/// different attendants wearing similar names.
#[must_use]
pub fn saved_row(entry: &SavedAttendantSummary, ctx: &jinn_picker::RowCtx<'_>) -> Line<'static> {
    let base = if ctx.is_selected {
        ratatui::style::Style::default().add_modifier(ratatui::style::Modifier::REVERSED)
    } else {
        ratatui::style::Style::default()
    };
    let mut spans = vec![Span::styled(entry.name.clone(), base.fg(Color::Cyan))];
    spans.push(Span::styled(
        format!(" · {} · {}", activation_label(entry), entry.trigger_label),
        base.fg(Color::DarkGray),
    ));
    if entry.pin_count > 0 {
        spans.push(Span::styled(
            format!(" · {} pinned", entry.pin_count),
            base.fg(Color::DarkGray),
        ));
    }
    Line::from(spans)
}

/// The activation's own word, matching the properties popup's labels.
#[must_use]
pub fn activation_label(entry: &SavedAttendantSummary) -> &'static str {
    match entry.activation {
        jinn_attendant_msg::AttendantActivation::Seed => "seed",
        jinn_attendant_msg::AttendantActivation::Reset => "reset",
        jinn_attendant_msg::AttendantActivation::Preserve => "preserve",
    }
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
