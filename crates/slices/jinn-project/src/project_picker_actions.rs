//! The project picker's behavior, as pure functions over its cell.
//!
//! The picker has no state that outlives the menu itself, so there is no
//! snapshot to restore: removing a project (`<c-d>`) writes through to
//! preferences immediately, and both confirm paths hand off to the session
//! lifecycle, which owns the scope transition.

use jinn_picker::PickerEntry;
use jinn_project_msg::ProjectEntry;
use jinn_project_msg::ProjectPickerState;
use jinn_theme::Theme;

type List = ProjectPickerState;

/// Wraps raw entries in the picker's row/search hooks.
///
/// The row renderer is the one the kernel used to reach through the spec: the
/// tilde-compressed display line, themed from the entry itself.
fn wrap(entries: Vec<ProjectEntry>) -> Vec<PickerEntry<ProjectEntry>> {
    jinn_picker::make_items_with_hooks(
        entries,
        jinn_picker::PickerItemHooks::new()
            .row(|entry: &ProjectEntry, ctx: &jinn_picker::RowCtx| {
                jinn_project_msg::render_project_row(
                    &entry.display,
                    ctx.is_selected,
                    ctx.match_ranges,
                    &entry.theme,
                )
            })
            .search(|entry: &ProjectEntry| entry.display.clone()),
    )
}

/// Resets the filter and selection, then loads one row per curated project
/// directory. `display` is precomputed so filtering and sorting operate on
/// what the user sees.
pub fn open(
    state: &mut List,
    projects: &[jinn_preferences_config::schemas::ProjectConfig],
    theme: &Theme,
) {
    state.selection.reset();
    let entries: Vec<ProjectEntry> = jinn_project_msg::project_entries(projects, theme);
    state.selection.set_items(wrap(entries));
}

/// The highlighted project's path, if a row is highlighted.
#[must_use]
pub fn selected_path(state: &List) -> Option<std::path::PathBuf> {
    state
        .selection
        .selected_item()
        .map(|item| item.entry().path.clone())
}

/// The highlighted project's entry, if a row is highlighted.
#[must_use]
pub fn selected_entry(state: &List) -> Option<PickerEntry<ProjectEntry>> {
    state.selection.selected_item().cloned()
}

/// `<c-d>`: drop the highlighted project from the remaining set, returning it
/// so the caller can persist the removal and the path so it can refresh.
///
/// Returns `None` when no row is highlighted, which is the only case where
/// the menu should do nothing.
pub fn remove_highlighted(state: &mut List) -> Option<std::path::PathBuf> {
    let entry = selected_entry(state)?;
    let path = entry.entry().path.clone();
    let remaining: Vec<ProjectEntry> = state
        .selection
        .items()
        .iter()
        .map(PickerEntry::entry)
        .filter(|candidate| candidate.path != path)
        .cloned()
        .collect();
    state.selection.set_items(wrap(remaining));
    Some(path)
}
