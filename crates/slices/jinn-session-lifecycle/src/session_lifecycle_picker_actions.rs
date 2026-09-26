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

//! The session-lifecycle picker's pure behavior, over its own cell.
//!
//! Every function here takes `&mut SessionLifecyclePickerState` and nothing
//! else: the rows, what a row contributes to the filter, and what confirming
//! one means. Actions that need app state live in `session_lifecycle_picker_routes`.

use jinn_picker::{PickerItemHooks, make_items_with_hooks};
use jinn_preferences_config::schemas::LifecycleCommand;
use jinn_session_lifecycle_msg::CommandTemplate;
use jinn_session_lifecycle_msg::picker_entry::{SessionLifecycleEntry, lifecycle_row};
use jinn_session_lifecycle_msg::picker_state::SessionLifecyclePickerState;
use jinn_theme::Theme;

/// Opens the picker over the configured lifecycles.
///
/// Always leads with the implicit blank lifecycle — the "new empty session"
/// row is not in `jinn.toml`, it is what you get with no configuration.
///
/// Called by the picker's scope-enter hook, on every entry, so each open
/// starts from a clean filter and highlight.
pub fn open(
    state: &mut SessionLifecyclePickerState,
    lifecycles: &[jinn_preferences_config::schemas::SessionLifecycle],
    theme: &Theme,
) {
    state.selection.reset();
    state.theme = theme.clone();
    state
        .selection
        .set_items(wrap_entries(build_entries(lifecycles, theme)));
}

/// The highlighted lifecycle's name and whether its setup needs parameters.
#[must_use]
pub fn highlighted(state: &SessionLifecyclePickerState) -> Option<(String, bool)> {
    state
        .selection
        .selected_item()
        .map(|item| (item.entry().name.clone(), item.entry().has_args))
}

/// The search text for one lifecycle row: name plus description.
///
/// An absent description contributes nothing but the separator space.
#[must_use]
pub fn search_text(entry: &SessionLifecycleEntry) -> String {
    match &entry.description {
        Some(desc) => format!("{} {desc}", entry.name),
        None => entry.name.clone(),
    }
}

/// The setup command template for `name`, when it is a shell command that
/// takes `$`-parameters.
///
/// This is the branch that hands off to the argument popup: only a shell
/// command has a template to fill in, and only a parameterized one needs the
/// user to supply values before setup can run.
#[must_use]
pub fn setup_template(
    lifecycles: &[jinn_preferences_config::schemas::SessionLifecycle],
    name: &str,
) -> Option<CommandTemplate> {
    lifecycles
        .iter()
        .find(|lifecycle| lifecycle.name == name)
        .and_then(|lifecycle| lifecycle.setup.as_ref())
        .and_then(|command| match command {
            LifecycleCommand::Shell(shell) => Some(CommandTemplate::parse(shell)),
            LifecycleCommand::Builtin(_) => None,
        })
}

/// Builds the rows: the implicit blank lifecycle, then every configured one.
///
/// `has_args` is detected from the setup command's template parameters, so the
/// row knows up front whether confirming it will ask for values.
#[must_use]
pub fn build_entries(
    lifecycles: &[jinn_preferences_config::schemas::SessionLifecycle],
    theme: &Theme,
) -> Vec<SessionLifecycleEntry> {
    let mut entries = vec![SessionLifecycleEntry {
        name: "blank".to_owned(),
        description: Some("New empty session".to_owned()),
        has_args: false,
        theme: theme.clone(),
    }];

    for lifecycle in lifecycles {
        let has_args = lifecycle
            .setup
            .as_ref()
            .and_then(|cmd| match cmd {
                LifecycleCommand::Shell(shell) => Some(shell.as_str()),
                LifecycleCommand::Builtin(_) => None,
            })
            .is_some_and(|cmd| CommandTemplate::parse(cmd).has_params());
        entries.push(SessionLifecycleEntry {
            name: lifecycle.name.clone(),
            description: lifecycle.description.clone(),
            has_args,
            theme: theme.clone(),
        });
    }

    entries
}

/// Wraps entries for the selection widget, wiring the row renderer and the
/// text the filter matches against.
///
/// Without these hooks the widget draws an empty label on every row, so they
/// are part of the picker's definition rather than decoration.
fn wrap_entries(
    entries: Vec<SessionLifecycleEntry>,
) -> Vec<jinn_picker::PickerEntry<SessionLifecycleEntry>> {
    make_items_with_hooks(
        entries,
        PickerItemHooks::new()
            .row(lifecycle_row)
            .search(search_text),
    )
}
