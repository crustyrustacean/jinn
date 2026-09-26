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

//! The task-list picker's pure behavior, over its own cell.
//!
//! A read-only tree browser: the active session's task list with phases as
//! roots and tasks as children. Postponed tasks are hidden, matching the
//! sidebar. Enter is a no-op — task management happens through the task tools.

use jinn_picker::{PickerItemHooks, make_items_with_hooks};
use jinn_selection_widget::TreeItem;
use jinn_theme::Theme;
use jinn_tools_msg::TaskList;
use jinn_tools_msg::TaskListPickerState;
use jinn_tools_msg::TaskListTreeEntry;
use jinn_tools_msg::TaskStatus;

/// Opens the picker over the given phases, as a fresh unfiltered tree.
///
/// An empty task list is fine: the browser shows an empty tree rather than
/// refusing to open, because "no tasks" is a legitimate thing to look at.
pub fn open(state: &mut TaskListPickerState, phases: &TaskList, theme: &Theme) {
    let entries = build_tree_entries(phases, theme);
    open_with_entries(state, entries);
}

/// Rebuilds the tree from the current phases, keeping the filter.
pub fn reload(state: &mut TaskListPickerState, phases: &TaskList, theme: &Theme) {
    let entries = build_tree_entries(phases, theme);
    state.tree.set_items(wrap_entries(entries));
}

/// Replaces the tree wholesale, from a fresh filter and highlight.
pub fn open_with_entries(state: &mut TaskListPickerState, entries: Vec<TaskListTreeEntry>) {
    state.tree = jinn_selection_widget::TreePickerState::new();
    state.tree.set_items(wrap_entries(entries));
}

/// Forgets every row, leaving an empty tree.
pub fn clear(state: &mut TaskListPickerState) {
    state.tree = jinn_selection_widget::TreePickerState::new();
}

/// Flattens the phase/task tree into the rows the widget shows.
///
/// Phases become roots and their tasks become children, so the widget can
/// indent and collapse them. Postponed tasks are dropped, matching the
/// sidebar: a postponed task is deliberately out of view, and showing it in a
/// browser would contradict that.
///
/// The theme is cloned per entry because every row carries its own palette —
/// a picker that renders with a theme different from the one its entries were
/// built with would show stale colors.
#[must_use]
pub fn build_tree_entries(phases: &TaskList, theme: &Theme) -> Vec<TaskListTreeEntry> {
    phases
        .phases()
        .iter()
        .flat_map(|phase| {
            let phase_id = format!("phase:{}", phase.id());
            let root = TaskListTreeEntry::new_phase(
                phase_id.clone(),
                phase.description().to_owned(),
                theme.clone(),
            );
            let children: Vec<TaskListTreeEntry> = phase
                .tasks()
                .iter()
                .filter(|task| task.status() != TaskStatus::Postponed)
                .map(|task| {
                    TaskListTreeEntry::new_task(
                        format!("task:{}", task.id()),
                        Some(phase_id.clone()),
                        task.description().to_owned(),
                        task.status(),
                        theme.clone(),
                    )
                })
                .collect();
            std::iter::once(root).chain(children)
        })
        .collect()
}

/// The text the filter matches a row by.
#[must_use]
pub fn search_text(entry: &TaskListTreeEntry) -> String {
    entry.display_label().to_owned()
}

/// Renders one row: the status-colored task row, with the widget's tree
/// connector prepended for children.
#[must_use]
pub fn task_list_row(
    entry: &TaskListTreeEntry,
    ctx: &jinn_picker::RowCtx<'_>,
) -> ratatui::text::Line<'static> {
    use ratatui::text::Span;

    let mut line = jinn_tools_msg::render_task_list_row(
        entry.display_label(),
        entry.row_status(),
        ctx.is_selected,
        ctx.match_ranges,
        entry.theme(),
    );
    if !ctx.tree_prefix.is_empty() {
        let mut spans = vec![Span::styled(ctx.tree_prefix.to_owned(), ctx.tree_style)];
        spans.append(&mut line.spans);
        line = ratatui::text::Line::from(spans);
    }
    line
}

/// Wraps entries for the tree widget, wiring the row renderer and the text the
/// filter matches against.
fn wrap_entries(
    entries: Vec<TaskListTreeEntry>,
) -> Vec<jinn_picker::PickerEntry<TaskListTreeEntry>> {
    make_items_with_hooks(
        entries,
        PickerItemHooks::new()
            .row(task_list_row)
            .search(search_text),
    )
}
