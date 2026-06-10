//! Picker intent handlers - navigation, filtering, confirmation, and scope toggling.
//!
//! Handles all picker intents: open, insert char, backspace, confirm, move up/down,
//! cursor movement, and keymap scope filter toggle. The `handle_picker_confirm`
//! function returns `(IntentResult, Option<Intent>)` to allow the caller
//! (`jinn-intent`) to re-dispatch keymap intents without creating a circular
//! dependency.

use crate::common::app_state::AppState;
use crate::common::app_state::FocusScope;
use crate::feat::context::protocol::command::{LoadPersonaPickerEntries, ScanContextFiles};
use crate::feat::preferences_actor::protocol::app_state_command::{AppStateUpdate, UpdateAppState};
use crate::feat::provider::protocol::command::{
    LoadProviderPickerEntries, ProviderSwitch, RescanPromptTemplates,
};
use crate::feat::session::model_selection::ModelSelection;
use crate::feat::session::protocol::load_session_picker_entries::LoadSessionPickerEntries;
use crate::feat::session::protocol::session_load_requested::SessionLoadRequested;
use crate::feat::skills::ScanSkills;
use crate::feat::tools_actor::tool_entry::ToolEntry;

use crate::feat::ui::picker_states::PickerExt;
use crate::protocol::{ChatEntry, Command, Intent, IntentResult, PickerKind};

use super::validator;

/// Maximum number of visible result rows for picker scroll clamping.
const PICKER_MAX_VISIBLE: usize = 100;

/// Opens a picker of the given kind. Sets mode to Picker and optionally
/// requests picker entries from the actor system.
pub fn handle_open_picker(state: &mut AppState, kind: PickerKind) -> IntentResult {
    if validator::validate_open_picker(state, &kind).is_err() {
        return IntentResult::empty();
    }

    state.frontend.scope_stack.push(FocusScope::Picker { kind });

    match kind {
        PickerKind::Provider => {
            state.provider.provider_picker.reset();
        }
        PickerKind::Session => {
            state.frontend.session_picker_mut().reset();
        }
        PickerKind::Persona => {
            state.frontend.persona_picker_mut().reset();
        }
        PickerKind::Theme => {
            state.frontend.theme_picker_mut().reset();
            // Save current theme so ESC can restore it.
            *state.frontend.theme_preview_original_mut() = Some(state.frontend.theme.clone());
            // Load discovered themes as entries.
            load_theme_picker_entries(state);
        }
        PickerKind::SessionLifecycle => {
            state.frontend.session_lifecycle_picker_mut().reset();
        }
        PickerKind::Plugin => {
            state.frontend.plugin_picker_mut().reset();
        }

        PickerKind::CompactionModel => {
            state.frontend.compaction_model_picker_mut().reset();
        }
        PickerKind::Tool => {
            state.frontend.tool_picker_mut().reset();
            // Snapshot current disabled tools for ESC revert.
            *state.frontend.tool_picker_snapshot_mut() =
                Some(state.active_session().disabled_tools().clone());
            load_tool_picker_entries(state);
        }
        PickerKind::Skill => {
            state.frontend.skill_picker_mut().reset();
            // Snapshot current disabled skills for ESC revert.
            *state.frontend.skill_picker_snapshot_mut() =
                Some(state.active_session().disabled_skills().clone());
            load_skill_picker_entries(state);
        }
        PickerKind::TaskList => {
            state.frontend.task_list_picker_mut().reset();
            load_task_list_picker_entries(state);
        }
    }

    match kind {
        PickerKind::Provider => {
            IntentResult::with_commands(vec![Command::LoadProviderPickerEntries(
                LoadProviderPickerEntries,
            )])
        }
        PickerKind::Session => {
            IntentResult::with_commands(vec![Command::LoadSessionPickerEntries(
                LoadSessionPickerEntries,
            )])
        }
        PickerKind::Persona => {
            IntentResult::with_commands(vec![Command::LoadPersonaPickerEntries(
                LoadPersonaPickerEntries,
            )])
        }
        PickerKind::Theme | PickerKind::Tool | PickerKind::Skill | PickerKind::TaskList => {
            IntentResult::empty()
        }
        PickerKind::SessionLifecycle => {
            // Populate from user preferences + implicit blank lifecycle.
            load_lifecycle_picker_entries(state);
            IntentResult::empty()
        }
        PickerKind::Plugin => {
            // Populate from discovered Lua plugins.
            load_plugin_picker_entries(state);
            IntentResult::empty()
        }

        PickerKind::CompactionModel => {
            IntentResult::with_commands(vec![Command::LoadCompactionModelPickerEntries(
                crate::feat::provider::protocol::command::LoadCompactionModelPickerEntries,
            )])
        }
    }
}

fn load_plugin_picker_entries(state: &mut AppState) {
    use crate::feat::plugin_dispatch::picker_entry::PluginPickerEntry;
    use crate::feat::theme::default_theme;

    let mut entries = Vec::new();

    for plugin in &state.discovered_plugins {
        entries.push(PluginPickerEntry {
            name: plugin.name.clone(),
            description: plugin.description.clone(),
            theme: default_theme(),
        });
    }

    state.frontend.pickers.plugin_picker.set_items(entries);
}

/// Loads discovered themes into the theme picker.
fn load_theme_picker_entries(state: &mut AppState) {
    use crate::feat::theme::ThemeEntry;

    let mut entries = Vec::new();

    // Always include the default (built-in) theme.
    entries.push(ThemeEntry {
        name: "default".to_owned(),
        theme: crate::feat::theme::default_theme(),
    });

    // Add discovered theme files from both user and system directories.
    let themes_dir = state.frontend.themes_dir.clone();
    let system_themes_dir = state.frontend.system_themes_dir.clone();
    if let Ok(discovered) = crate::feat::theme::discover_themes(&themes_dir) {
        for (name, _path) in discovered {
            if name == "default" {
                continue; // skip duplicate
            }
            if let Ok(theme) =
                crate::feat::theme::load_theme(&name, &themes_dir, &system_themes_dir)
            {
                entries.push(ThemeEntry { name, theme });
            }
        }
    }
    // Also discover themes that only exist in the system directory.
    if let Ok(system_discovered) = crate::feat::theme::discover_themes(&system_themes_dir) {
        for (name, _path) in system_discovered {
            if name == "default" {
                continue;
            }
            // Skip if already loaded from user dir.
            if entries.iter().any(|e: &ThemeEntry| e.name == name) {
                continue;
            }
            if let Ok(theme) =
                crate::feat::theme::load_theme(&name, &themes_dir, &system_themes_dir)
            {
                entries.push(ThemeEntry { name, theme });
            }
        }
    }

    entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    state.frontend.theme_picker_mut().set_items(entries);
}

/// Previews the selected theme in real-time when the Theme picker is active.
fn preview_theme_if_active(state: &mut AppState) {
    if state.frontend.scope_stack.picker_kind() != Some(&PickerKind::Theme) {
        return;
    }
    if let Some(entry) = state.frontend.theme_picker().selected_item() {
        state.frontend.theme = entry.theme.clone();
        state.invalidate_theme_caches();
    }
}

/// Resets the preview scroll offset to 0 when the skill picker is active.
fn reset_preview_scroll(state: &mut AppState) {
    if state.frontend.scope_stack.picker_kind() == Some(&PickerKind::Skill) {
        state.frontend.set_skill_preview_scroll(0);
    }
}

/// Scrolls the preview pane up by one page.
pub fn handle_preview_scroll_up(state: &mut AppState) -> IntentResult {
    let page_size = preview_page_size(state);
    state.frontend.set_skill_preview_scroll(
        state
            .frontend
            .skill_preview_scroll()
            .saturating_sub(page_size),
    );
    IntentResult::empty()
}

/// Scrolls the preview pane down by one page.
pub fn handle_preview_scroll_down(state: &mut AppState) -> IntentResult {
    let page_size = preview_page_size(state);
    state.frontend.set_skill_preview_scroll(
        state
            .frontend
            .skill_preview_scroll()
            .saturating_add(page_size),
    );
    IntentResult::empty()
}

/// Returns the number of visible rows in the preview pane.
///
/// Computed from the popup height minus chrome (border, input, separator).
fn preview_page_size(state: &AppState) -> usize {
    // Use a reasonable default; exact size depends on terminal.
    // The popup is ~60% of terminal height, minus border (2), input (1), separator (1).
    let _ = state;
    10
}

/// Inserts a character into the active picker's filter.
pub fn handle_insert_char(state: &mut AppState, ch: char) -> IntentResult {
    validator::validate_picker_insert_char(state, ch);
    if let Some(picker) = state.active_picker_ops() {
        picker.insert_char(ch);
    }
    IntentResult::empty()
}

/// Handles `PasteText` in picker scope - bulk inserts pasted text into the filter.
///
/// Newlines are stripped by the picker's `insert_text` method since the filter
/// is a single-line input.
pub fn handle_picker_paste(state: &mut AppState, text: &str) -> IntentResult {
    if let Some(picker) = state.active_picker_ops() {
        picker.insert_text(text);
    }
    IntentResult::empty()
}

/// Removes the last character from the active picker's filter.
pub fn handle_backspace(state: &mut AppState) -> IntentResult {
    validator::validate_picker_backspace(state);
    if let Some(picker) = state.active_picker_ops() {
        picker.backspace();
    }
    IntentResult::empty()
}

/// Confirms the active picker selection.
///
/// Returns `(IntentResult, Option<Intent>)`. For Provider and
/// Session pickers, the second element is `None`. For Keymap picker, returns
/// `(IntentResult::empty(), Some(selected_intent))` so the caller can re-dispatch.
pub fn handle_picker_confirm(state: &mut AppState) -> (IntentResult, Option<Intent>) {
    if validator::validate_picker_confirm(state).is_err() {
        return (IntentResult::empty(), None);
    }

    match state.frontend.scope_stack.picker_kind().copied() {
        Some(PickerKind::Provider) => (confirm_provider(state), None),
        Some(PickerKind::Session) => (confirm_session(state), None),
        Some(PickerKind::Persona) => (confirm_persona(state), None),
        Some(PickerKind::Theme) => (confirm_theme(state), None),
        Some(PickerKind::SessionLifecycle) => (confirm_session_lifecycle(state), None),
        Some(PickerKind::Plugin) => (confirm_plugin(state), None),

        Some(PickerKind::CompactionModel | PickerKind::TaskList) | None => {
            (IntentResult::empty(), None)
        }
        Some(PickerKind::Tool) => (confirm_tool(state), None),
        Some(PickerKind::Skill) => (confirm_skill(state), None),
    }
}

/// Moves the selection up in the active picker.
pub fn handle_move_up(state: &mut AppState) -> IntentResult {
    validator::validate_picker_move_up(state);
    if let Some(picker) = state.active_picker_ops() {
        picker.move_up(PICKER_MAX_VISIBLE);
    }
    reset_preview_scroll(state);
    preview_theme_if_active(state);
    IntentResult::empty()
}

/// Moves the selection down in the active picker.
pub fn handle_move_down(state: &mut AppState) -> IntentResult {
    validator::validate_picker_move_down(state);
    if let Some(picker) = state.active_picker_ops() {
        picker.move_down(PICKER_MAX_VISIBLE);
    }
    reset_preview_scroll(state);
    preview_theme_if_active(state);
    IntentResult::empty()
}

/// Moves the filter cursor left in the active picker.
pub fn handle_move_cursor_left(state: &mut AppState) -> IntentResult {
    validator::validate_picker_move_cursor_left(state);
    if let Some(picker) = state.active_picker_ops() {
        picker.move_cursor_left();
    }
    IntentResult::empty()
}

/// Moves the filter cursor right in the active picker.
pub fn handle_move_cursor_right(state: &mut AppState) -> IntentResult {
    validator::validate_picker_move_cursor_right(state);
    if let Some(picker) = state.active_picker_ops() {
        picker.move_cursor_right();
    }
    IntentResult::empty()
}

// --- Private confirm handlers ---

/// Confirms the selected provider and dispatches a switch command.
fn confirm_provider(state: &mut AppState) -> IntentResult {
    let Some(entry) = state.provider.provider_picker.selected_item() else {
        return IntentResult::empty();
    };
    if !entry.is_available {
        return IntentResult::empty();
    }
    let provider_id = entry.provider_id.clone();
    let session_id = state.session.active_session_id().clone();

    state.frontend.scope_stack.pop();
    IntentResult::with_commands(vec![
        Command::ProviderSwitch(ProviderSwitch {
            session_id,
            provider_id: ModelSelection::Single(provider_id.clone()),
        }),
        Command::UpdateAppState(UpdateAppState {
            updates: vec![AppStateUpdate::SetLastModel(Some(provider_id))],
        }),
    ])
}

/// Confirms the selected persona and sets it as active.
fn confirm_persona(state: &mut AppState) -> IntentResult {
    let Some(entry) = state.frontend.persona_picker().selected_item() else {
        return IntentResult::empty();
    };
    let persona_name = entry.name.clone();

    // Find the matching persona and set it as active.
    let persona = state
        .context
        .personas
        .iter()
        .find(|p| p.name == persona_name)
        .cloned();
    if let Some(p) = persona {
        state.context.active_persona = Some(p);
    }

    // Also update the active session's persona binding.
    state
        .active_session_mut()
        .set_persona_name(persona_name.clone());

    state.frontend.scope_stack.pop();

    IntentResult::with_commands(vec![Command::UpdateAppState(UpdateAppState {
        updates: vec![AppStateUpdate::SetPersona(Some(persona_name))],
    })])
}

/// Confirms the selected theme and persists it to preferences.
fn confirm_theme(state: &mut AppState) -> IntentResult {
    let Some(entry) = state.frontend.theme_picker().selected_item() else {
        return IntentResult::empty();
    };
    let theme_name = entry.name.clone();

    // Theme is already previewed (set on move). Just persist.
    *state.frontend.theme_preview_original_mut() = None;
    state.frontend.scope_stack.pop();

    IntentResult::with_commands(vec![Command::UpdateAppState(UpdateAppState {
        updates: vec![AppStateUpdate::SetTheme(Some(theme_name))],
    })])
}

/// Confirms the selected session and dispatches a switch command.
fn confirm_session(state: &mut AppState) -> IntentResult {
    let Some(entry) = state.frontend.session_picker().selected_item() else {
        return IntentResult::empty();
    };
    let session_id = entry.session_id.clone();

    state.session.begin_load(session_id.clone());
    state.frontend.scope_stack.pop();

    IntentResult::with_commands(vec![Command::SessionLoadRequested(SessionLoadRequested {
        session_id,
    })])
}

/// Populates the lifecycle picker entries from user preferences.
///
/// Always includes the implicit blank lifecycle (no commands, uses default CWD)
/// as the first entry, followed by all lifecycles defined in `jinn.toml`.
fn load_lifecycle_picker_entries(state: &mut AppState) {
    use crate::feat::session_lifecycle::command_template::CommandTemplate;
    use crate::feat::session_lifecycle::picker_entry::SessionLifecycleEntry;

    let mut entries = Vec::new();

    let theme = state.frontend.theme.clone();

    // Always include the implicit blank lifecycle.
    entries.push(SessionLifecycleEntry {
        name: "blank".to_owned(),
        description: Some("New empty session".to_owned()),
        has_args: false,
        theme: theme.clone(),
    });

    // Add lifecycles from preferences.
    for lifecycle in &state.frontend.preferences.session_lifecycles {
        let has_args = lifecycle
            .setup
            .as_ref()
            .and_then(|cmd| match cmd {
                crate::feat::session_lifecycle::builtin::LifecycleCommand::Shell(s) => {
                    Some(s.as_str())
                }
                crate::feat::session_lifecycle::builtin::LifecycleCommand::Builtin(_) => None,
            })
            .is_some_and(|cmd| CommandTemplate::parse(cmd).has_params());
        entries.push(SessionLifecycleEntry {
            name: lifecycle.name.clone(),
            description: lifecycle.description.clone(),
            has_args,
            theme: theme.clone(),
        });
    }

    state
        .frontend
        .session_lifecycle_picker_mut()
        .set_items(entries);
}

/// Confirms the selected session lifecycle.
///
/// If the lifecycle has args, the arg input popup would open (Phase 6).
/// For now, directly triggers setup with empty args (lifecycles without args)
/// or with empty args as a placeholder.
fn confirm_session_lifecycle(state: &mut AppState) -> IntentResult {
    let Some(entry) = state.frontend.session_lifecycle_picker().selected_item() else {
        return IntentResult::empty();
    };

    let lifecycle_name = entry.name.clone();
    let has_args = entry.has_args;
    state.frontend.scope_stack.pop();

    if has_args {
        // Save context and open the arg input popup.
        let template_display = state
            .frontend
            .preferences
            .session_lifecycles
            .iter()
            .find(|l| l.name == lifecycle_name)
            .and_then(|l| l.setup.as_ref())
            .and_then(|cmd| match cmd {
                crate::feat::session_lifecycle::builtin::LifecycleCommand::Shell(s) => {
                    Some(s.as_str())
                }
                crate::feat::session_lifecycle::builtin::LifecycleCommand::Builtin(_) => None,
            })
            .map(|cmd| {
                crate::feat::session_lifecycle::command_template::CommandTemplate::parse(cmd)
                    .display()
            })
            .unwrap_or_default();

        state.frontend.arg_input = crate::common::app_state::ArgInputState {
            lifecycle_name,
            template_display,
            text: crate::common::line_input::LineInput::new(),
        };
        state
            .frontend
            .scope_stack
            .push(crate::common::app_state::FocusScope::ArgInput);
        return IntentResult::empty();
    }

    // No args - proceed directly.
    crate::feat::session_lifecycle::intent::handle_session_lifecycle_setup(
        state,
        &lifecycle_name,
        &[],
    )
}

/// Confirms the selected plugin, starts it, and switches to the Plugin tab.
fn confirm_plugin(state: &mut AppState) -> IntentResult {
    let Some(entry) = state.frontend.plugin_picker().selected_item() else {
        return IntentResult::empty();
    };
    let script = entry.name.clone();

    // Pop picker overlay.
    state.frontend.scope_stack.pop();

    let session_id = state.session.active_session_id().clone();
    IntentResult::with_commands(vec![Command::AttachPlugin(
        crate::feat::plugin_dispatch::protocol::command::AttachPlugin {
            session_id,
            plugin_name: script.clone(),
        },
    )])
}

/// Marks each entry as enabled/disabled based on the session's `disabled_tools` set.
fn load_tool_picker_entries(state: &mut AppState) {
    let disabled = state.active_session().disabled_tools();
    let theme = state.frontend.theme.clone();

    let mut entries: Vec<ToolEntry> = state
        .context
        .tool_definitions
        .values()
        .map(|def| {
            let name = def.name.clone();
            let description = def.description.clone();
            ToolEntry {
                name: name.clone(),
                description: description.clone(),
                search_text: format!("{name} {description}"),
                enabled: !disabled.contains(&def.name),
                theme: theme.clone(),
            }
        })
        .collect();

    entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    state.frontend.tool_picker_mut().set_items(entries);
}

/// Confirms the tool picker: collects disabled tool names from picker entries
/// and writes them to the active session's profile.
fn confirm_tool(state: &mut AppState) -> IntentResult {
    let disabled: std::collections::HashSet<String> = state
        .frontend
        .tool_picker()
        .items()
        .iter()
        .filter(|entry| !entry.enabled)
        .map(|entry| entry.name.clone())
        .collect();

    state.active_session_mut().set_disabled_tools(disabled);
    *state.frontend.tool_picker_snapshot_mut() = None;
    state.frontend.scope_stack.pop();
    IntentResult::empty()
}

/// Toggles the `enabled` state of the currently selected tool entry.
pub fn handle_tool_toggle(state: &mut AppState) -> IntentResult {
    state.frontend.tool_picker_mut().with_selected_mut(|entry| {
        entry.enabled = !entry.enabled;
    });
    state
        .frontend
        .tool_picker_mut()
        .move_down(PICKER_MAX_VISIBLE);
    IntentResult::empty()
}

/// Populates the skill picker entries from discovered skills.
///
/// Delegates to [`crate::feat::skills::reload::reload_skill_picker_entries`].
fn load_skill_picker_entries(state: &mut AppState) {
    crate::feat::skills::reload::reload_skill_picker_entries(state);
}

/// Populates the task list picker entries from the active session's task list.
///
/// Phases are emitted as tree roots; tasks are emitted as children of their owning
/// phase (parent_id = the phase's id string). Postponed tasks are filtered out,
/// matching the sidebar's `render_text` behavior.
///
/// Empty task lists produce an empty picker - no panic.
fn load_task_list_picker_entries(state: &mut AppState) {
    use crate::feat::theme::default_theme;
    use crate::feat::todo_list::TaskStatus;
    use crate::feat::todo_list::picker_entry::TaskListTreeEntry;

    let theme = default_theme();
    let entries: Vec<TaskListTreeEntry> = state
        .active_session()
        .task_list()
        .phases()
        .iter()
        .flat_map(|phase| {
            let phase_id_str = format!("phase:{}", phase.id());
            let phase_entry = TaskListTreeEntry::new_phase(
                phase_id_str.clone(),
                phase.description().to_owned(),
                theme.clone(),
            );
            let task_entries: Vec<TaskListTreeEntry> = phase
                .tasks()
                .iter()
                .filter(|task| task.status() != TaskStatus::Postponed)
                .map(|task| {
                    TaskListTreeEntry::new_task(
                        format!("task:{}", task.id()),
                        Some(phase_id_str.clone()),
                        task.description().to_owned(),
                        task.status(),
                        theme.clone(),
                    )
                })
                .collect();
            std::iter::once(phase_entry).chain(task_entries)
        })
        .collect();

    state.frontend.task_list_picker_mut().set_items(entries);
}

/// Confirms the skill picker: collects disabled skill names from picker entries
/// and writes them to the active session's profile.
fn confirm_skill(state: &mut AppState) -> IntentResult {
    let disabled: std::collections::HashSet<String> = state
        .frontend
        .skill_picker()
        .items()
        .iter()
        .filter(|entry| !entry.enabled)
        .map(|entry| entry.name.clone())
        .collect();

    state.active_session_mut().set_disabled_skills(disabled);
    *state.frontend.skill_picker_snapshot_mut() = None;
    state.frontend.scope_stack.pop();
    IntentResult::empty()
}

/// Toggles the `enabled` state of the currently selected skill entry.
pub fn handle_skill_toggle(state: &mut AppState) -> IntentResult {
    state
        .frontend
        .skill_picker_mut()
        .with_selected_mut(|entry| {
            entry.enabled = !entry.enabled;
        });
    state
        .frontend
        .skill_picker_mut()
        .move_down(PICKER_MAX_VISIBLE);
    IntentResult::empty()
}

/// Refreshes discovered project resources (skills, prompts, AGENTS.md) by
/// rescanning the session's cwd. Issues all three scan commands so the
/// discovery coordinator receives a complete set of `*Loaded` events and
/// settles cleanly (rather than arming the 3000ms safety-net timer on a
/// partial trigger). The scan actors handle the actual I/O and reload picker
/// entries.
///
/// No-op unless the skill picker is the active scope.
pub fn handle_refresh_skills(state: &mut AppState) -> IntentResult {
    if state.frontend.scope_stack.picker_kind() != Some(&PickerKind::Skill) {
        return IntentResult::empty();
    }

    state
        .active_session_mut()
        .push_entry(ChatEntry::transient("Refreshing project resources..."));

    let session_id = state.active_session().session_id().clone();

    IntentResult::with_commands(vec![
        Command::ScanSkills(ScanSkills {
            session_id: session_id.clone(),
        }),
        Command::RescanPromptTemplates(RescanPromptTemplates {
            session_id: session_id.clone(),
        }),
        Command::ScanContextFiles(ScanContextFiles { session_id }),
    ])
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use super::*;
    use crate::feat::session::ChatSessionState;
    use crate::feat::todo_list::TaskStatus;
    use crate::feat::todo_list::picker_entry::RowStatus;
    use crate::protocol::ChatEntryKind;
    use jinn_selection_widget::TreeItem;
    use std::path::PathBuf;

    #[rstest::rstest]
    fn confirm_provider_rejects_unavailable() {
        // Kills: delete ! in confirm_provider.
        // If the ! were deleted, unavailable providers could be confirmed.
        let mut state = AppState::default();
        let origin = ChatSessionState::new();
        state.session.insert(origin);
        state
            .session
            .set_active(state.session.active_session_id().clone());

        // Add an unavailable provider entry to the picker and select it.
        let entry = crate::protocol::PickerEntry {
            provider_id: "openrouter/gpt-4".to_owned(),
            name: "openrouter".to_owned(),
            provider_name: "openrouter".to_owned(),
            backend: "openrouter".to_owned(),
            model: "gpt-4".to_owned(),
            search_text: "gpt-4 openrouter".to_owned(),
            is_alias: false,
            alias_target: None,
            is_available: false, // Unavailable!
            is_remote: false,
            is_active: false,
            theme: crate::feat::theme::default_theme(),
        };
        state.provider.provider_picker.set_items(vec![entry]);
        state.provider.provider_picker.move_down(1); // Select first entry.

        let result = confirm_provider(&mut state);

        // Then no commands are emitted (the unavailable provider was rejected).
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn confirm_provider_accepts_available() {
        // Counter-test: confirms that available providers ARE accepted.
        let mut state = AppState::default();
        let origin = ChatSessionState::new();
        state.session.insert(origin);
        state
            .session
            .set_active(state.session.active_session_id().clone());

        let entry = crate::protocol::PickerEntry {
            provider_id: "ollama/llama3".to_owned(),
            name: "ollama".to_owned(),
            provider_name: "ollama".to_owned(),
            backend: "ollama".to_owned(),
            model: "llama3".to_owned(),
            search_text: "llama3 ollama".to_owned(),
            is_alias: false,
            alias_target: None,
            is_available: true, // Available!
            is_remote: false,
            is_active: false,
            theme: crate::feat::theme::default_theme(),
        };
        state.provider.provider_picker.set_items(vec![entry]);
        state.provider.provider_picker.move_down(1);

        let result = confirm_provider(&mut state);

        // Then commands are emitted.
        assert!(!result.commands.is_empty());
    }

    #[rstest::rstest]
    fn confirm_persona_sets_correct_persona() {
        // Kills: replace == with != in confirm_persona.
        // If the match were inverted, the wrong persona would be set.
        use crate::feat::persona::PersonaEntry;

        let mut state = AppState::default();
        let origin = ChatSessionState::new();
        state.session.insert(origin);
        state
            .session
            .set_active(state.session.active_session_id().clone());

        // Add two personas to context.
        state.context.personas = vec![
            crate::feat::persona::Persona {
                name: "coder".to_owned(),
                description: String::new(),
                body: "You are a coder.".to_owned(),
                file_path: PathBuf::new(),
            },
            crate::feat::persona::Persona {
                name: "writer".to_owned(),
                description: String::new(),
                body: "You are a writer.".to_owned(),
                file_path: PathBuf::new(),
            },
        ];

        // Set picker entries with "writer" as the selected item.
        let entries = vec![
            PersonaEntry {
                name: "coder".to_owned(),
                description: String::new(),
                is_active: false,
                theme: crate::feat::theme::default_theme(),
            },
            PersonaEntry {
                name: "writer".to_owned(),
                description: String::new(),
                is_active: false,
                theme: crate::feat::theme::default_theme(),
            },
        ];
        state.frontend.persona_picker_mut().set_items(entries);
        state.frontend.persona_picker_mut().move_down(1); // coder
        state.frontend.persona_picker_mut().move_down(1); // writer

        let result = confirm_persona(&mut state);

        // Then the active persona is "writer", not "coder".
        assert_eq!(
            state
                .context
                .active_persona
                .as_ref()
                .map(|p| p.name.as_str()),
            Some("writer"),
            "confirm_persona should set the correct persona"
        );
        assert!(!result.commands.is_empty());
    }

    #[rstest::rstest]
    fn confirm_session_lifecycle_finds_correct_lifecycle_for_args() {
        // Kills: replace == with != in confirm_session_lifecycle.
        // If the match were inverted, find() would locate the WRONG lifecycle,
        // producing the wrong template_display in the arg_input state.
        let mut state = AppState::default();
        let origin = ChatSessionState::new();
        state.session.insert(origin);
        state
            .session
            .set_active(state.session.active_session_id().clone());

        // Add two lifecycles with args ($1) to preferences.
        state.frontend.preferences.session_lifecycles = vec![
            crate::feat::preferences_actor::user_preferences::SessionLifecycle {
                name: "project-a".to_owned(),
                description: None,
                setup: Some(
                    crate::feat::session_lifecycle::builtin::LifecycleCommand::Shell(
                        "cd /a/$1".to_owned(),
                    ),
                ),
                teardown: None,
            },
            crate::feat::preferences_actor::user_preferences::SessionLifecycle {
                name: "project-b".to_owned(),
                description: None,
                setup: Some(
                    crate::feat::session_lifecycle::builtin::LifecycleCommand::Shell(
                        "cd /b/$1".to_owned(),
                    ),
                ),
                teardown: None,
            },
        ];

        // Load entries and select "project-b".
        load_lifecycle_picker_entries(&mut state);
        state.frontend.session_lifecycle_picker_mut().move_down(1); // blank
        state.frontend.session_lifecycle_picker_mut().move_down(1); // project-a
        state.frontend.session_lifecycle_picker_mut().move_down(1); // project-b

        let _result = confirm_session_lifecycle(&mut state);

        // Then the arg_input state references "project-b" and its template.
        assert_eq!(state.frontend.arg_input.lifecycle_name, "project-b");
        assert!(
            state.frontend.arg_input.template_display.contains("/b/"),
            "template_display should contain /b/ from project-b's setup command, got: {}",
            state.frontend.arg_input.template_display,
        );
    }

    #[rstest::rstest]
    fn load_lifecycle_picker_entries_populates_picker() {
        // Kills: replace load_lifecycle_picker_entries with ().
        // If the function were a no-op, the picker would remain empty.
        let mut state = AppState::default();

        // Add lifecycle entries to preferences.
        state.frontend.preferences.session_lifecycles = vec![
            crate::feat::preferences_actor::user_preferences::SessionLifecycle {
                name: "project-a".to_owned(),
                description: Some("Project A setup".to_owned()),
                setup: None,
                teardown: None,
            },
        ];

        // When loading lifecycle picker entries.
        load_lifecycle_picker_entries(&mut state);

        // Then the picker has entries (blank + project-a = 2).
        let items = state.frontend.session_lifecycle_picker().items();
        assert_eq!(
            items.len(),
            2,
            "should have blank + 1 lifecycle = 2 entries"
        );
        assert_eq!(items[0].name, "blank");
        assert_eq!(items[1].name, "project-a");
    }

    // --- Skill picker tests ---

    fn setup_state_with_skills() -> AppState {
        use crate::feat::skills::Skill;
        use std::path::PathBuf;

        let mut state = AppState::default();
        let origin = ChatSessionState::new();
        state.session.insert(origin);
        state
            .session
            .set_active(state.session.active_session_id().clone());

        state.active_session_mut().set_discovered_skills(vec![
            Skill {
                name: "phased-task-loop".to_owned(),
                description: "Structured phased implementation workflow".to_owned(),
                body: String::new(),
                file_path: PathBuf::from("/tmp/skills/phased-task-loop/SKILL.md"),
                base_dir: PathBuf::from("/tmp/skills/phased-task-loop"),
                source: crate::feat::skills::SkillSource::Global,
            },
            Skill {
                name: "web-coder".to_owned(),
                description: "Expert web development".to_owned(),
                body: String::new(),
                file_path: PathBuf::from("/tmp/skills/web-coder/SKILL.md"),
                base_dir: PathBuf::from("/tmp/skills/web-coder"),
                source: crate::feat::skills::SkillSource::Global,
            },
        ]);

        state
    }

    #[rstest::rstest]
    fn load_skill_picker_entries_populates_picker() {
        // Given state with two skills.
        let mut state = setup_state_with_skills();

        // When loading skill picker entries.
        load_skill_picker_entries(&mut state);

        // Then the picker has two entries.
        let items = state.frontend.skill_picker().items();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name, "phased-task-loop");
        assert_eq!(items[1].name, "web-coder");
    }

    #[rstest::rstest]
    fn opening_skill_picker_preserves_preview_cache() {
        use jinn_selection_widget::PreviewCache;

        // Given a populated cache (simulating prior viewing).
        let mut state = setup_state_with_skills();
        state.frontend.caches.skill_preview_cache.write().insert(
            "web-coder".to_owned(),
            80,
            vec![ratatui::text::Line::raw("rendered")],
        );
        assert_eq!(state.frontend.caches.skill_preview_cache.read().len(), 1);

        // When the skill picker is opened.
        handle_open_picker(&mut state, PickerKind::Skill);

        // Then the cache is preserved (bodies haven't changed).
        assert_eq!(state.frontend.caches.skill_preview_cache.read().len(), 1);
    }

    #[rstest::rstest]
    fn load_skill_picker_entries_marks_disabled() {
        // Given state with "web-coder" disabled.
        let mut state = setup_state_with_skills();
        state
            .active_session_mut()
            .set_disabled_skills(std::collections::HashSet::from(["web-coder".to_owned()]));

        // When loading skill picker entries.
        load_skill_picker_entries(&mut state);

        // Then "web-coder" is marked disabled.
        let items = state.frontend.skill_picker().items();
        assert!(items[0].enabled, "phased-task-loop should be enabled");
        assert!(!items[1].enabled, "web-coder should be disabled");
    }

    #[rstest::rstest]
    fn confirm_skill_writes_disabled_set() {
        // Given an open skill picker with "web-coder" toggled off.
        let mut state = setup_state_with_skills();
        load_skill_picker_entries(&mut state);

        // Select "web-coder" (second entry) and toggle it off.
        state.frontend.skill_picker_mut().move_down(1); // move from 0 → 1
        handle_skill_toggle(&mut state);

        // When confirming.
        let _ = confirm_skill(&mut state);

        // Then the session's disabled_skills contains "web-coder".
        let disabled = state.active_session().disabled_skills().clone();
        assert_eq!(
            disabled,
            std::collections::HashSet::from(["web-coder".to_owned()])
        );
    }

    #[rstest::rstest]
    fn confirm_skill_clears_snapshot() {
        // Given an open skill picker with a snapshot.
        let mut state = setup_state_with_skills();
        *state.frontend.skill_picker_snapshot_mut() = Some(std::collections::HashSet::new());
        load_skill_picker_entries(&mut state);

        // When confirming.
        let _ = confirm_skill(&mut state);

        // Then the snapshot is cleared.
        assert!(state.frontend.skill_picker_snapshot().is_none());
    }

    #[rstest::rstest]
    fn handle_skill_toggle_flips_enabled() {
        // Given an open skill picker.
        let mut state = setup_state_with_skills();
        load_skill_picker_entries(&mut state);

        // The first entry (phased-task-loop) is selected by default (selection=0).
        assert!(state.frontend.skill_picker().items()[0].enabled);

        // When toggling.
        handle_skill_toggle(&mut state);

        // Then the first entry is now disabled.
        assert!(!state.frontend.skill_picker().items()[0].enabled);

        // And toggling again re-enables it (cursor moved to entry 1,
        // so we go back up first).
        state.frontend.skill_picker_mut().move_up(1);
        handle_skill_toggle(&mut state);
        assert!(state.frontend.skill_picker().items()[0].enabled);
    }

    #[rstest::rstest]
    fn handle_skill_toggle_moves_cursor_down() {
        // Given an open skill picker with two entries.
        let mut state = setup_state_with_skills();
        load_skill_picker_entries(&mut state);

        // Selection starts at 0.
        assert_eq!(state.frontend.skill_picker().selection(), 0);

        // When toggling.
        handle_skill_toggle(&mut state);

        // Then the cursor has moved down to 1.
        assert_eq!(state.frontend.skill_picker().selection(), 1);
    }

    // --- Plugin picker tests ---

    fn setup_state_with_plugins() -> AppState {
        use crate::common::app_state::DiscoveredPlugin;

        let mut state = AppState::default();
        let origin = ChatSessionState::new();
        state.session.insert(origin);
        state
            .session
            .set_active(state.session.active_session_id().clone());

        state.discovered_plugins = vec![
            DiscoveredPlugin {
                name: "judge-fail".to_owned(),
                description: Some("Runs judge on failure".to_owned()),
            },
            DiscoveredPlugin {
                name: "consensus".to_owned(),
                description: Some("Multi-model consensus".to_owned()),
            },
        ];

        state
    }

    #[rstest::rstest]
    fn load_plugin_picker_entries_populates_from_discovered_plugins() {
        // Given state with two discovered plugins.
        let mut state = setup_state_with_plugins();

        // When loading plugin picker entries.
        load_plugin_picker_entries(&mut state);

        // Then the picker has two entries matching the plugins.
        let items = state.frontend.plugin_picker().items();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name, "judge-fail");
        assert_eq!(
            items[0].description,
            Some("Runs judge on failure".to_owned())
        );
        assert_eq!(items[1].name, "consensus");
        assert_eq!(
            items[1].description,
            Some("Multi-model consensus".to_owned())
        );
    }

    #[rstest::rstest]
    fn load_plugin_picker_entries_empty_when_no_plugins() {
        // Given state with no discovered plugins.
        let mut state = AppState::default();
        let origin = ChatSessionState::new();
        state.session.insert(origin);
        state
            .session
            .set_active(state.session.active_session_id().clone());

        // When loading plugin picker entries.
        load_plugin_picker_entries(&mut state);

        // Then the picker is empty.
        let items = state.frontend.plugin_picker().items();
        assert!(items.is_empty());
    }

    #[rstest::rstest]
    fn confirm_plugin_emits_attach_plugin_with_lua_config() {
        // Given a plugin picker populated with plugins.
        let mut state = setup_state_with_plugins();
        load_plugin_picker_entries(&mut state);
        // Select the second entry (consensus).
        state.frontend.plugin_picker_mut().move_down(1);
        state.frontend.plugin_picker_mut().move_down(1);
        // Push a picker scope so the pop has something to remove.
        state.frontend.scope_stack.push(FocusScope::Picker {
            kind: PickerKind::Plugin,
        });

        // When confirming.
        let result = confirm_plugin(&mut state);

        // Then an AttachPlugin command is emitted with the selected plugin name.
        assert_eq!(result.commands.len(), 1);
        let cmd = &result.commands[0];
        let Command::AttachPlugin(attach) = cmd else {
            panic!("expected AttachPlugin, got {cmd:?}");
        };
        assert_eq!(attach.plugin_name, "consensus");
    }

    #[rstest::rstest]
    fn confirm_plugin_pops_picker_scope() {
        // Given a plugin picker with a Picker scope on the stack.
        let mut state = setup_state_with_plugins();
        load_plugin_picker_entries(&mut state);
        // Select an entry.
        state.frontend.plugin_picker_mut().move_down(1);
        state.frontend.scope_stack.push(FocusScope::Picker {
            kind: PickerKind::Plugin,
        });

        // When confirming.
        let _ = confirm_plugin(&mut state);

        // Then the Picker scope was popped.
        assert!(!matches!(
            state.frontend.scope_stack.current(),
            FocusScope::Picker { .. }
        ));
    }

    #[rstest::rstest]
    fn refresh_skills_posts_transient_message() {
        // Given state with skills and the skill picker active.
        let mut state = setup_state_with_skills();
        load_skill_picker_entries(&mut state);
        state.frontend.scope_stack.push(FocusScope::Picker {
            kind: PickerKind::Skill,
        });

        // When handling RefreshSkills.
        let _result = handle_refresh_skills(&mut state);

        // Then a transient message was posted.
        let last = state
            .active_session()
            .history()
            .last()
            .expect("should have entry");
        assert!(matches!(last.kind, ChatEntryKind::Transient(_)));
    }

    #[rstest::rstest]
    fn refresh_skills_returns_scan_commands_for_all_resources() {
        // Given state with skills and the skill picker active.
        let mut state = setup_state_with_skills();
        load_skill_picker_entries(&mut state);
        state.frontend.scope_stack.push(FocusScope::Picker {
            kind: PickerKind::Skill,
        });

        // When handling RefreshSkills.
        let result = handle_refresh_skills(&mut state);

        // Then all three scan commands are returned so discovery settles cleanly.
        assert!(
            result
                .commands
                .iter()
                .any(|c| matches!(c, Command::ScanSkills(..))),
            "expected ScanSkills command"
        );
        assert!(
            result
                .commands
                .iter()
                .any(|c| matches!(c, Command::RescanPromptTemplates(..))),
            "expected RescanPromptTemplates command"
        );
        assert!(
            result
                .commands
                .iter()
                .any(|c| matches!(c, Command::ScanContextFiles(..))),
            "expected ScanContextFiles command"
        );
    }

    #[rstest::rstest]
    fn refresh_skills_noop_when_skill_picker_not_active() {
        // Given state without the skill picker active.
        let mut state = AppState::default();

        // When handling RefreshSkills.
        let result = handle_refresh_skills(&mut state);

        // Then no commands and no messages.
        assert!(result.commands.is_empty());
    }

    fn setup_state_with_task_list() -> (AppState, crate::feat::todo_list::TaskId) {
        use crate::feat::todo_list::TaskPosition;

        let mut state = AppState::default();
        let mut origin = ChatSessionState::new();

        // Phase 1 with 2 tasks (one Pending, one Completed).
        let phase1 = origin.task_list_mut().add_phase("Research");
        let _ = origin
            .task_list_mut()
            .add_task(&phase1, "Read codebase", TaskPosition::End);
        let task2 = origin
            .task_list_mut()
            .add_task(&phase1, "Write notes", TaskPosition::End)
            .expect("add_task");
        origin
            .task_list_mut()
            .complete_task(&task2)
            .expect("complete");

        // Phase 2 with 3 tasks (Pending, Cancelled, plus a Postponed source).
        // postpone_task marks the source as Postponed AND inserts a new Pending
        // copy with the same description, so we surface the source ID to tests.
        let phase2 = origin.task_list_mut().add_phase("Build");
        let _ = origin
            .task_list_mut()
            .add_task(&phase2, "Implement feature", TaskPosition::End);
        let task_cancel = origin
            .task_list_mut()
            .add_task(&phase2, "Investigate alt", TaskPosition::End)
            .expect("add_task");
        origin
            .task_list_mut()
            .cancel_task(&task_cancel)
            .expect("cancel");
        let to_postpone = origin
            .task_list_mut()
            .add_task(&phase2, "Refactor later", TaskPosition::End)
            .expect("add_task");
        let postponed_id = to_postpone.clone();
        origin
            .task_list_mut()
            .postpone_task(&to_postpone, TaskPosition::After(task_cancel.clone()))
            .expect("postpone");

        let origin_id = origin.session_id().clone();
        state.session.insert(origin);
        assert!(
            state.session.set_active(origin_id),
            "origin session must be present for set_active"
        );
        (state, postponed_id)
    }

    #[rstest::rstest]
    fn load_task_list_picker_entries_skips_postponed() {
        // Given a session with one postponed task among other tasks.
        // postpone_task creates a new Pending copy with the same description, so we
        // must verify the *source* (Postponed) entry is excluded by ID, not by label.
        let (mut state, postponed_id) = setup_state_with_task_list();

        // When loading task list picker entries.
        load_task_list_picker_entries(&mut state);

        // Then no entry has the postponed task's ID.
        let excluded_id = format!("task:{postponed_id}");
        let items = state.frontend.task_list_picker().items();
        assert!(
            items.iter().all(|e| e.id() != excluded_id),
            "postponed source task should not appear in picker (id={excluded_id})"
        );
        // Sanity: the new Pending copy with the same description IS present.
        assert!(
            items.iter().any(|e| e.display_label() == "Refactor later"),
            "Pending copy of postponed task should be visible"
        );
    }

    #[rstest::rstest]
    fn load_task_list_picker_entries_produces_correct_tree_shape() {
        // Given a session with two phases and mixed-status tasks.
        let (mut state, _postponed_id) = setup_state_with_task_list();

        // When loading.
        load_task_list_picker_entries(&mut state);

        // Then there are exactly 2 phase roots.
        let items = state.frontend.task_list_picker().items();
        let roots: Vec<_> = items.iter().filter(|e| e.parent_id().is_none()).collect();
        assert_eq!(roots.len(), 2, "should have 2 phase roots");
        assert_eq!(roots[0].display_label(), "Research");
        assert_eq!(roots[1].display_label(), "Build");

        // And each task's parent_id matches its phase's id.
        let phase_ids: Vec<&str> = roots.iter().map(|e| e.id()).collect();
        for item in items.iter().filter(|e| e.parent_id().is_some()) {
            assert!(
                phase_ids.contains(&item.parent_id().expect("task parent")),
                "task {:?} should reference a known phase id",
                item.display_label()
            );
        }

        // And the counts match: Phase 1 -> 2 tasks; Phase 2 -> 3 tasks (Pending,
        // Cancelled, and the Pending copy created by postpone_task).
        let research_children: Vec<_> = items
            .iter()
            .filter(|e| e.parent_id() == Some(phase_ids[0]))
            .collect();
        let build_children: Vec<_> = items
            .iter()
            .filter(|e| e.parent_id() == Some(phase_ids[1]))
            .collect();
        assert_eq!(research_children.len(), 2);
        assert_eq!(build_children.len(), 3);
    }

    #[rstest::rstest]
    fn load_task_list_picker_entries_carries_status_through() {
        // Given a session with completed and cancelled tasks.
        let (mut state, _postponed_id) = setup_state_with_task_list();

        // When loading.
        load_task_list_picker_entries(&mut state);

        // Then task rows carry their status in row_status.

        let items = state.frontend.task_list_picker().items();
        let statuses: Vec<_> = items
            .iter()
            .filter_map(|e| match e.row_status() {
                RowStatus::Task(s) => Some((e.display_label(), s)),
                RowStatus::Phase => None,
            })
            .collect();

        let by_label: std::collections::HashMap<&str, TaskStatus> =
            statuses.iter().map(|(l, s)| (*l, *s)).collect();
        assert_eq!(
            by_label.get("Write notes").copied(),
            Some(TaskStatus::Completed),
            "'Write notes' should be Completed"
        );
        assert_eq!(
            by_label.get("Investigate alt").copied(),
            Some(TaskStatus::Cancelled),
            "'Investigate alt' should be Cancelled"
        );
        assert_eq!(
            by_label.get("Read codebase").copied(),
            Some(TaskStatus::Pending),
            "'Read codebase' should be Pending"
        );
        // The Pending copy of the postponed task should also carry its status.
        assert_eq!(
            by_label.get("Refactor later").copied(),
            Some(TaskStatus::Pending),
            "'Refactor later' (Pending copy) should be Pending"
        );
    }

    #[rstest::rstest]
    fn load_task_list_picker_entries_empty_task_list_no_panic() {
        // Given a default session with an empty task list.
        let mut state = AppState::default();

        // When loading.
        load_task_list_picker_entries(&mut state);

        // Then the picker is empty and nothing panicked.
        assert!(state.frontend.task_list_picker().items().is_empty());
    }

    #[rstest::rstest]
    fn handle_picker_confirm_task_list_is_noop_and_keeps_scope() {
        // Given state with the TaskList picker scope on the stack.
        let (mut state, _postponed_id) = setup_state_with_task_list();
        state.frontend.scope_stack.push(FocusScope::Picker {
            kind: PickerKind::TaskList,
        });
        let len_before = state.frontend.scope_stack.len();

        // When confirming.
        let (result, follow_up) = handle_picker_confirm(&mut state);

        // Then no commands, no follow-up, and the scope stack is unchanged.
        assert!(result.commands.is_empty(), "no commands");
        assert!(follow_up.is_none(), "no follow-up");
        assert_eq!(
            state.frontend.scope_stack.len(),
            len_before,
            "scope stack must remain unchanged on no-op confirm"
        );
        assert!(matches!(
            state.frontend.scope_stack.current(),
            FocusScope::Picker {
                kind: PickerKind::TaskList
            }
        ));
    }

    #[rstest::rstest]
    fn esc_from_task_list_picker_restores_sidebar_task_list_scope() {
        // Given a scope stack like: [Normal, SidebarTaskList, Picker(TaskList)].
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::SidebarTaskList);
        state.frontend.scope_stack.push(FocusScope::Picker {
            kind: PickerKind::TaskList,
        });

        // When Esc is pressed.
        let _ = crate::feat::chat_input::intent::handle_enter_normal_mode(&mut state);

        // Then we should return to SidebarTaskList, not Normal.
        assert!(
            matches!(
                state.frontend.scope_stack.current(),
                FocusScope::SidebarTaskList
            ),
            "Esc from TaskList picker should restore SidebarTaskList scope, got: {:?}",
            state.frontend.scope_stack.current()
        );
    }
}
