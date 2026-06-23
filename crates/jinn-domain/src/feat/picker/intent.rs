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
use crate::feat::preferences_actor::protocol::command::{PreferenceUpdate, UpdatePreferences};
use crate::feat::provider::ProviderState;
use crate::feat::provider::picker_entry::PickerEntry;
use crate::feat::provider::protocol::command::{
    LoadProviderPickerEntries, ProviderSwitch, RescanPromptTemplates,
};
use crate::feat::session::model_selection::{AlloyStrategy, ModelSelection};
use crate::feat::session::protocol::load_session_picker_entries::LoadSessionPickerEntries;
use crate::feat::session::protocol::mark_session_interacted::MarkSessionInteracted;
use crate::feat::session::protocol::session_load_requested::SessionLoadRequested;
use crate::feat::skills::ScanSkills;
use crate::feat::tools_actor::tool_entry::ToolEntry;

use crate::feat::ui::picker_states::PickerExt;
use crate::protocol::{ChatEntry, Intent, IntentResult, PickerKind};

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
            // Derive alloy mode from the active session's model selection:
            // an existing Alloy opens in alloy mode (with members pre-checked
            // by the loader), anything else opens in single mode.
            state.provider.alloy_mode = matches!(
                state.active_session().profile().model,
                ModelSelection::Alloy { .. }
            );
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

        PickerKind::ReasoningEffort => {
            state.frontend.reasoning_effort_picker_mut().reset();
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
        PickerKind::Project => {
            // Defensive: a stale override from an abandoned previous flow
            // must never leak into a new project-picker session.
            state.frontend.pending_session_cwd = None;
            state.frontend.project_picker_mut().reset();
            load_project_picker_entries(state);
        }
    }

    match kind {
        PickerKind::Provider => IntentResult::with_message(LoadProviderPickerEntries),
        PickerKind::Session => IntentResult::with_message(LoadSessionPickerEntries),
        PickerKind::Persona => IntentResult::with_message(LoadPersonaPickerEntries),
        PickerKind::Theme
        | PickerKind::Tool
        | PickerKind::Skill
        | PickerKind::TaskList
        | PickerKind::Project => IntentResult::empty(),
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

        PickerKind::CompactionModel => IntentResult::with_message(
            crate::feat::provider::protocol::command::LoadCompactionModelPickerEntries,
        ),

        PickerKind::ReasoningEffort => IntentResult::with_message(
            crate::feat::provider::protocol::command::LoadReasoningEffortPickerEntries,
        ),
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

    entries.sort_by_key(|e| e.name.to_lowercase());

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
        Some(PickerKind::Project) => (confirm_project(state), None),
        Some(PickerKind::ReasoningEffort) => (confirm_reasoning_effort(state), None),

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

/// Confirms the selected provider and dispatches a switch command.
///
/// In single mode, ENTER commits the highlighted entry as a single model.
/// In alloy mode, ENTER force-includes the highlighted entry (deduped against the
/// checked set) and commits: one model -> `Single`, two or more -> anonymous
/// `Alloy`. The highlighted entry must be available or nothing is committed.
fn confirm_provider(state: &mut AppState) -> IntentResult {
    // Resolve the highlighted entry. It is the foundation of both modes, and
    // its availability gates the entire confirm.
    let Some(highlight) = state.provider.provider_picker.selected_item() else {
        return IntentResult::empty();
    };
    if !highlight.is_available {
        return IntentResult::empty();
    }
    let highlight_id = highlight.provider_id.clone();

    let model_selection = resolve_provider_selection(&state.provider, highlight_id);

    let last_model = Some(model_selection.clone());
    let session_id = state.session.active_session_id().clone();

    state.frontend.scope_stack.pop();
    IntentResult::empty()
        .message(ProviderSwitch {
            session_id,
            provider_id: model_selection,
        })
        .message(UpdateAppState {
            updates: vec![AppStateUpdate::SetLastModel(last_model)],
        })
}

/// Resolves the provider confirmation decision for the given highlighted entry.
///
/// Single mode: the highlighted entry becomes `ModelSelection::Single`.
/// Alloy mode: the checked set union the highlight (deduped); one model -> `Single`,
/// two or more -> `Alloy`.
fn resolve_provider_selection(provider: &ProviderState, highlighted: String) -> ModelSelection {
    if !provider.alloy_mode {
        return ModelSelection::Single(highlighted);
    }
    let mut models: Vec<String> = provider
        .provider_picker
        .items()
        .iter()
        .filter(|e| e.selected && e.is_available)
        .map(|e| e.provider_id.clone())
        .collect();

    // Force-include the highlighted entry (ENTER adds it before committing).
    if !models.contains(&highlighted) {
        models.push(highlighted);
    }

    if models.len() <= 1 {
        ModelSelection::Single(models.into_iter().next().unwrap_or_default())
    } else {
        ModelSelection::Alloy {
            models,
            strategy: AlloyStrategy::RoundRobin { index: 0 },
        }
    }
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
    let session_id = state.session.active_session_id().clone();
    state
        .active_session_mut()
        .set_persona_name(persona_name.clone());

    state.frontend.scope_stack.pop();

    IntentResult::with_message(UpdateAppState {
        updates: vec![AppStateUpdate::SetPersona(Some(persona_name))],
    })
    .message(MarkSessionInteracted { session_id })
}

/// Confirms the selected reasoning effort and applies it.
///
/// Dual-write mirroring [`confirm_persona`]: sets the active session's
/// `reasoning_effort` override in-memory, persists it immediately via
/// [`MarkSessionInteracted`], and updates the global default via
/// [`UpdatePreferences`] so new sessions inherit the choice.
fn confirm_reasoning_effort(state: &mut AppState) -> IntentResult {
    let Some(entry) = state.frontend.reasoning_effort_picker().selected_item() else {
        return IntentResult::empty();
    };
    let effort = entry.effort;
    let session_id = state.session.active_session_id().clone();

    state.active_session_mut().profile_mut().reasoning_effort = Some(effort);

    state.frontend.scope_stack.pop();

    IntentResult::empty()
        .message(MarkSessionInteracted { session_id })
        .message(UpdatePreferences {
            updates: vec![PreferenceUpdate::SetDefaultReasoningEffort(Some(effort))],
        })
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

    IntentResult::with_message(UpdateAppState {
        updates: vec![AppStateUpdate::SetTheme(Some(theme_name))],
    })
}

/// Confirms the selected session and dispatches a switch command.
fn confirm_session(state: &mut AppState) -> IntentResult {
    let Some(entry) = state.frontend.session_picker().selected_item() else {
        return IntentResult::empty();
    };
    let session_id = entry.session_id.clone();

    state.session.begin_load(session_id.clone());
    state.frontend.scope_stack.pop();

    IntentResult::with_message(SessionLoadRequested { session_id })
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
        None,
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
    IntentResult::with_message(
        crate::feat::plugin_dispatch::protocol::command::AttachPlugin {
            session_id,
            plugin_name: script,
        },
    )
}

/// Marks each entry as enabled/disabled based on the session's `disabled_tools` set.
fn load_tool_picker_entries(state: &mut AppState) {
    let active_session = state.active_session();
    let disabled = active_session.disabled_tools();
    let provider_name = active_session.model_selection().provider_name().to_owned();
    let theme = state.frontend.theme.clone();

    let active_id = state.session.active_session_id().clone();
    let mut entries: Vec<ToolEntry> = state
        .context
        .tools_for_session(&active_id)
        .into_iter()
        .filter(|def| def.available_for_provider(&provider_name))
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

    entries.sort_by_key(|e| e.name.to_lowercase());

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

/// Populates the project picker entries from `UserPreferences.projects`.
///
/// Entries are pre-computed display strings (tilde-compressed) so the picker
/// never has to call `shorten_path` per-render.
pub(crate) fn load_project_picker_entries(state: &mut AppState) {
    use crate::feat::project::picker_entry::{ProjectEntry, project_entries};

    let theme = state.frontend.theme.clone();
    let entries: Vec<ProjectEntry> = project_entries(&state.frontend.preferences.projects, &theme);
    state.frontend.project_picker_mut().set_items(entries);
}

/// Confirms the highlighted project: stashes its dir as the pending session
/// CWD, pops the picker, and delegates to a plain new session.
///
/// `Enter` path: the new session inherits nothing from the active session's
/// CWD - it is created at the chosen project dir via the override channel.
fn confirm_project(state: &mut AppState) -> IntentResult {
    let Some(entry) = state.frontend.project_picker().selected_item() else {
        return IntentResult::empty();
    };

    // Stash the chosen dir and pop the picker before delegating, so the new
    // session is created in Normal scope at the right CWD.
    state.frontend.pending_session_cwd = Some(entry.path.clone());
    state.frontend.scope_stack.pop();

    crate::feat::session_lifecycle::intent::handle_session_lifecycle_setup(state, "", &[], None)
}

/// Confirms the highlighted project and chains into the lifecycle picker.
///
/// `<c-enter>` path: same dir-stash + pop as `confirm_project`, but then opens
/// the session lifecycle picker so the user picks a recipe (and optional args).
/// The stashed dir survives the lifecycle -> arg-input chain because every
/// confirmation path in that chain consumes `pending_session_cwd`.
pub fn handle_project_lifecycle_confirm(state: &mut AppState) -> IntentResult {
    let Some(entry) = state.frontend.project_picker().selected_item() else {
        return IntentResult::empty();
    };

    state.frontend.pending_session_cwd = Some(entry.path.clone());
    state.frontend.scope_stack.pop();

    // Re-enter the lifecycle picker. `handle_open_picker` pushes a fresh
    // `Picker { SessionLifecycle }` scope.
    handle_open_picker(state, PickerKind::SessionLifecycle)
}

/// Removes the highlighted project from the curated list (`d`).
///
/// Applies the diff optimistically to `frontend.preferences.projects`,
/// reloads the picker items so the list updates immediately, and emits
/// `UpdatePreferences` so the `PreferencesActor` persists the change.
/// The `PreferencesStateSyncActor` will overwrite `frontend.preferences` with
/// the same data when the broadcast round-trips.
pub fn handle_project_remove_highlighted(state: &mut AppState) -> IntentResult {
    let Some(entry) = state.frontend.project_picker().selected_item().cloned() else {
        return IntentResult::empty();
    };

    state
        .frontend
        .preferences
        .projects
        .retain(|p| p.path != entry.path);
    load_project_picker_entries(state);

    IntentResult::with_message(UpdatePreferences {
        updates: vec![PreferenceUpdate::RemoveProject(entry.path)],
    })
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

/// Toggles the selected model's `selected` state in the provider picker.
///
/// Used for multi-select alloy building. When toggled on, the entry gets a
/// checkmark and is sorted to the top of the list. The cursor stays put so the
/// user can toggle several adjacent entries without losing their place.
pub fn handle_model_toggle(state: &mut AppState) -> IntentResult {
    // No-op outside alloy mode: single mode never builds checkmarks.
    if !state.provider.alloy_mode {
        return IntentResult::empty();
    }
    state.provider.provider_picker.with_selected_mut(|entry| {
        entry.selected = !entry.selected;
    });

    // Re-sort: selected entries to top, then alphabetical.
    resort_provider_picker(&mut state.provider.provider_picker);

    IntentResult::empty()
}
/// Toggles the provider picker between single-model and alloy-selection modes.
///
/// No-op unless the provider picker is active. When entering alloy mode, the
/// current session model's entries are pre-checked (so editing an existing alloy
/// only requires swapping the desired members). When leaving, all checks are
/// cleared. Either way the list is re-sorted so checked entries float to the top.
pub fn handle_toggle_alloy_mode(state: &mut AppState) -> IntentResult {
    // Flip first, then branch on the resulting (target) state.
    state.provider.alloy_mode = !state.provider.alloy_mode;

    if state.provider.alloy_mode {
        // Entered alloy mode: pre-check the current session model's entries,
        // so editing an existing alloy only requires swapping members.
        let model_selection = state.active_session().profile().model.clone();
        let mut entries = state.provider.provider_picker.items().to_vec();
        crate::feat::provider::loader::pre_check_active_models(&mut entries, &model_selection);
        state.provider.provider_picker.set_items(entries);
    } else {
        // Left alloy mode: clear every check.
        let mut entries = state.provider.provider_picker.items().to_vec();
        for entry in &mut entries {
            entry.selected = false;
        }
        state.provider.provider_picker.set_items(entries);
    }

    resort_provider_picker(&mut state.provider.provider_picker);
    IntentResult::empty()
}

/// Re-sorts provider picker entries: selected first (alphabetical), then unselected (alphabetical).
fn resort_provider_picker(picker: &mut jinn_selection_widget::SelectionState<PickerEntry>) {
    let entries: Vec<PickerEntry> = picker.items().to_vec();
    let (selected, unselected): (Vec<_>, Vec<_>) = entries.into_iter().partition(|e| e.selected);
    let mut sorted: Vec<PickerEntry> = selected;
    sorted.extend(unselected);
    picker.set_items(sorted);
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

    IntentResult::empty()
        .message(ScanSkills {
            session_id: session_id.clone(),
        })
        .message(RescanPromptTemplates {
            session_id: session_id.clone(),
        })
        .message(ScanContextFiles { session_id })
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
            selected: false,
            theme: crate::feat::theme::default_theme(),
        };
        state.provider.provider_picker.set_items(vec![entry]);
        state.provider.provider_picker.move_down(1); // Select first entry.

        let result = confirm_provider(&mut state);

        // Then no commands are emitted (the unavailable provider was rejected).
        assert!(result.message_names.is_empty());
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
            selected: false,
            theme: crate::feat::theme::default_theme(),
        };
        state.provider.provider_picker.set_items(vec![entry]);
        state.provider.provider_picker.move_down(1);

        let result = confirm_provider(&mut state);

        // Then commands are emitted.
        assert!(!result.message_names.is_empty());
    }

    #[rstest::rstest]
    fn handle_model_toggle_flips_selected() {
        // Given a picker with two available entries.
        let mut state = AppState::default();
        let origin = ChatSessionState::new();
        state.session.insert(origin);
        state
            .session
            .set_active(state.session.active_session_id().clone());
        state.frontend.scope_stack.push(FocusScope::Picker {
            kind: PickerKind::Provider,
        });

        let entries = vec![
            crate::protocol::PickerEntry {
                provider_id: "ollama/llama3".to_owned(),
                name: "ollama".to_owned(),
                provider_name: "ollama".to_owned(),
                backend: "ollama".to_owned(),
                model: "llama3".to_owned(),
                search_text: "llama3".to_owned(),
                is_alias: false,
                alias_target: None,
                is_available: true,
                is_remote: false,
                is_active: false,
                selected: false,
                theme: crate::feat::theme::default_theme(),
            },
            crate::protocol::PickerEntry {
                provider_id: "openrouter/gpt-4".to_owned(),
                name: "openrouter".to_owned(),
                provider_name: "openrouter".to_owned(),
                backend: "openrouter".to_owned(),
                model: "gpt-4".to_owned(),
                search_text: "gpt-4".to_owned(),
                is_alias: false,
                alias_target: None,
                is_available: true,
                is_remote: false,
                is_active: false,
                selected: false,
                theme: crate::feat::theme::default_theme(),
            },
        ];
        state.provider.provider_picker.set_items(entries);
        state.provider.alloy_mode = true;
        // Cursor starts on the first entry (selection 0) after reset.

        // When toggling model selection.
        handle_model_toggle(&mut state);

        // Then the first entry is now selected.
        let first = state.provider.provider_picker.items()[0].selected;
        assert!(first, "first entry should be selected after toggle");
    }

    #[rstest::rstest]
    fn handle_model_toggle_keeps_cursor_in_place() {
        // Given a picker with two entries, cursor on the first, in alloy mode.
        let mut state = AppState::default();
        let origin = ChatSessionState::new();
        state.session.insert(origin);
        state
            .session
            .set_active(state.session.active_session_id().clone());
        state.frontend.scope_stack.push(FocusScope::Picker {
            kind: PickerKind::Provider,
        });

        let entries = vec![
            crate::protocol::PickerEntry {
                provider_id: "ollama/llama3".to_owned(),
                name: "ollama".to_owned(),
                provider_name: "ollama".to_owned(),
                backend: "ollama".to_owned(),
                model: "llama3".to_owned(),
                search_text: "llama3".to_owned(),
                is_alias: false,
                alias_target: None,
                is_available: true,
                is_remote: false,
                is_active: false,
                selected: false,
                theme: crate::feat::theme::default_theme(),
            },
            crate::protocol::PickerEntry {
                provider_id: "openrouter/gpt-4".to_owned(),
                name: "openrouter".to_owned(),
                provider_name: "openrouter".to_owned(),
                backend: "openrouter".to_owned(),
                model: "gpt-4".to_owned(),
                search_text: "gpt-4".to_owned(),
                is_alias: false,
                alias_target: None,
                is_available: true,
                is_remote: false,
                is_active: false,
                selected: false,
                theme: crate::feat::theme::default_theme(),
            },
        ];
        state.provider.provider_picker.set_items(entries);
        state.provider.alloy_mode = true;
        // Cursor starts on the first entry (selection 0) after reset.
        assert_eq!(state.provider.provider_picker.selection(), 0);
        // When toggling model selection.
        handle_model_toggle(&mut state);

        // Then the cursor stays on the first entry.
        assert_eq!(
            state.provider.provider_picker.selection(),
            0,
            "cursor should not advance after toggle"
        );
    }

    #[rstest::rstest]
    fn handle_model_toggle_toggles_off() {
        // Given a picker with one entry already selected.
        let mut state = AppState::default();
        let origin = ChatSessionState::new();
        state.session.insert(origin);
        state
            .session
            .set_active(state.session.active_session_id().clone());
        state.frontend.scope_stack.push(FocusScope::Picker {
            kind: PickerKind::Provider,
        });

        let entries = vec![crate::protocol::PickerEntry {
            provider_id: "ollama/llama3".to_owned(),
            name: "ollama".to_owned(),
            provider_name: "ollama".to_owned(),
            backend: "ollama".to_owned(),
            model: "llama3".to_owned(),
            search_text: "llama3".to_owned(),
            is_alias: false,
            alias_target: None,
            is_available: true,
            is_remote: false,
            is_active: false,
            selected: true, // Already selected
            theme: crate::feat::theme::default_theme(),
        }];
        state.provider.provider_picker.set_items(entries);
        state.provider.alloy_mode = true;
        state.provider.provider_picker.move_down(1);

        // When toggling model selection again.
        handle_model_toggle(&mut state);

        // Then the entry is now deselected.
        let first = state.provider.provider_picker.items()[0].selected;
        assert!(!first, "entry should be deselected after second toggle");
    }

    #[rstest::rstest]
    fn open_picker_sets_single_mode_for_single_model_session() {
        // Given a session on a single model.
        let mut state = AppState::default();
        let origin = ChatSessionState::new();
        state.session.insert(origin);
        state
            .session
            .set_active(state.session.active_session_id().clone());
        state
            .active_session_mut()
            .set_model(ModelSelection::Single("ollama/llama3".to_owned()));

        // When opening the provider picker.
        handle_open_picker(&mut state, PickerKind::Provider);

        // Then alloy_mode is false (single mode).
        assert!(
            !state.provider.alloy_mode,
            "picker should open in single mode for a single-model session"
        );
    }

    #[rstest::rstest]
    fn open_picker_sets_alloy_mode_for_alloy_session() {
        // Given a session on an alloy of two models.
        let mut state = AppState::default();
        let origin = ChatSessionState::new();
        state.session.insert(origin);
        state
            .session
            .set_active(state.session.active_session_id().clone());
        state.active_session_mut().set_model(ModelSelection::Alloy {
            models: vec!["ollama/llama3".to_owned(), "openrouter/gpt-4".to_owned()],
            strategy: AlloyStrategy::RoundRobin { index: 0 },
        });

        // When opening the provider picker.
        handle_open_picker(&mut state, PickerKind::Provider);

        // Then alloy_mode is true (alloy mode).
        assert!(
            state.provider.alloy_mode,
            "picker should open in alloy mode for an alloy session"
        );
    }

    #[rstest::rstest]
    fn toggle_alloy_mode_flips_false_to_true() {
        // Given a provider picker in single mode.
        let mut state = AppState::default();
        let origin = ChatSessionState::new();
        state.session.insert(origin);
        state
            .session
            .set_active(state.session.active_session_id().clone());
        state.frontend.scope_stack.push(FocusScope::Picker {
            kind: PickerKind::Provider,
        });
        state.provider.alloy_mode = false;

        // When toggling alloy mode.
        handle_toggle_alloy_mode(&mut state);

        // Then alloy_mode is now true.
        assert!(state.provider.alloy_mode, "mode should flip to alloy");
    }

    #[rstest::rstest]
    fn toggle_alloy_mode_flips_true_to_false() {
        // Given a provider picker in alloy mode.
        let mut state = AppState::default();
        let origin = ChatSessionState::new();
        state.session.insert(origin);
        state
            .session
            .set_active(state.session.active_session_id().clone());
        state.frontend.scope_stack.push(FocusScope::Picker {
            kind: PickerKind::Provider,
        });
        state.provider.alloy_mode = true;

        // When toggling alloy mode.
        handle_toggle_alloy_mode(&mut state);

        // Then alloy_mode is now false.
        assert!(!state.provider.alloy_mode, "mode should flip to single");
    }

    #[rstest::rstest]
    fn toggle_into_alloy_mode_pre_checks_current_single_model() {
        // Given a provider picker in single mode with the session on a single model.
        let mut state = AppState::default();
        let origin = ChatSessionState::new();
        state.session.insert(origin);
        state
            .session
            .set_active(state.session.active_session_id().clone());
        state
            .active_session_mut()
            .set_model(ModelSelection::Single("ollama/llama3".to_owned()));
        state.frontend.scope_stack.push(FocusScope::Picker {
            kind: PickerKind::Provider,
        });
        let entries = vec![
            crate::protocol::PickerEntry {
                provider_id: "ollama/llama3".to_owned(),
                name: "ollama".to_owned(),
                provider_name: "ollama".to_owned(),
                backend: "ollama".to_owned(),
                model: "llama3".to_owned(),
                search_text: "llama3".to_owned(),
                is_alias: false,
                alias_target: None,
                is_available: true,
                is_remote: false,
                is_active: false,
                selected: false,
                theme: crate::feat::theme::default_theme(),
            },
            crate::protocol::PickerEntry {
                provider_id: "openrouter/gpt-4".to_owned(),
                name: "openrouter".to_owned(),
                provider_name: "openrouter".to_owned(),
                backend: "openrouter".to_owned(),
                model: "gpt-4".to_owned(),
                search_text: "gpt-4".to_owned(),
                is_alias: false,
                alias_target: None,
                is_available: true,
                is_remote: false,
                is_active: false,
                selected: false,
                theme: crate::feat::theme::default_theme(),
            },
        ];
        state.provider.provider_picker.set_items(entries);
        state.provider.alloy_mode = false;

        // When toggling into alloy mode.
        handle_toggle_alloy_mode(&mut state);

        // Then the entry matching the current model is pre-checked.
        let llama = state
            .provider
            .provider_picker
            .items()
            .iter()
            .find(|e| e.provider_id == "ollama/llama3")
            .expect("llama entry");
        assert!(
            llama.selected,
            "current model should be pre-checked on entering alloy mode"
        );
    }

    #[rstest::rstest]
    fn toggle_into_alloy_mode_pre_checks_all_alloy_members() {
        // Given a provider picker in single mode with the session on an alloy.
        let mut state = AppState::default();
        let origin = ChatSessionState::new();
        state.session.insert(origin);
        state
            .session
            .set_active(state.session.active_session_id().clone());
        state.active_session_mut().set_model(ModelSelection::Alloy {
            models: vec!["ollama/llama3".to_owned(), "openrouter/gpt-4".to_owned()],
            strategy: AlloyStrategy::RoundRobin { index: 0 },
        });
        state.frontend.scope_stack.push(FocusScope::Picker {
            kind: PickerKind::Provider,
        });
        let entries = vec![
            crate::protocol::PickerEntry {
                provider_id: "ollama/llama3".to_owned(),
                name: "ollama".to_owned(),
                provider_name: "ollama".to_owned(),
                backend: "ollama".to_owned(),
                model: "llama3".to_owned(),
                search_text: "llama3".to_owned(),
                is_alias: false,
                alias_target: None,
                is_available: true,
                is_remote: false,
                is_active: false,
                selected: false,
                theme: crate::feat::theme::default_theme(),
            },
            crate::protocol::PickerEntry {
                provider_id: "openrouter/gpt-4".to_owned(),
                name: "openrouter".to_owned(),
                provider_name: "openrouter".to_owned(),
                backend: "openrouter".to_owned(),
                model: "gpt-4".to_owned(),
                search_text: "gpt-4".to_owned(),
                is_alias: false,
                alias_target: None,
                is_available: true,
                is_remote: false,
                is_active: false,
                selected: false,
                theme: crate::feat::theme::default_theme(),
            },
        ];
        state.provider.provider_picker.set_items(entries);
        state.provider.alloy_mode = false;

        // When toggling into alloy mode.
        handle_toggle_alloy_mode(&mut state);

        // Then both alloy members are pre-checked.
        let checked: Vec<&str> = state
            .provider
            .provider_picker
            .items()
            .iter()
            .filter(|e| e.selected)
            .map(|e| e.provider_id.as_str())
            .collect();
        assert_eq!(checked.len(), 2, "both alloy members should be pre-checked");
    }

    #[rstest::rstest]
    fn toggle_out_of_alloy_mode_clears_all_checks() {
        // Given a provider picker in alloy mode with two entries checked.
        let mut state = AppState::default();
        let origin = ChatSessionState::new();
        state.session.insert(origin);
        state
            .session
            .set_active(state.session.active_session_id().clone());
        state.frontend.scope_stack.push(FocusScope::Picker {
            kind: PickerKind::Provider,
        });
        let entries = vec![
            crate::protocol::PickerEntry {
                provider_id: "ollama/llama3".to_owned(),
                name: "ollama".to_owned(),
                provider_name: "ollama".to_owned(),
                backend: "ollama".to_owned(),
                model: "llama3".to_owned(),
                search_text: "llama3".to_owned(),
                is_alias: false,
                alias_target: None,
                is_available: true,
                is_remote: false,
                is_active: false,
                selected: true,
                theme: crate::feat::theme::default_theme(),
            },
            crate::protocol::PickerEntry {
                provider_id: "openrouter/gpt-4".to_owned(),
                name: "openrouter".to_owned(),
                provider_name: "openrouter".to_owned(),
                backend: "openrouter".to_owned(),
                model: "gpt-4".to_owned(),
                search_text: "gpt-4".to_owned(),
                is_alias: false,
                alias_target: None,
                is_available: true,
                is_remote: false,
                is_active: false,
                selected: true,
                theme: crate::feat::theme::default_theme(),
            },
        ];
        state.provider.provider_picker.set_items(entries);
        state.provider.alloy_mode = true;

        // When toggling out of alloy mode.
        handle_toggle_alloy_mode(&mut state);

        // Then no entries remain checked.
        let any_checked = state
            .provider
            .provider_picker
            .items()
            .iter()
            .any(|e| e.selected);
        assert!(
            !any_checked,
            "all checks should be cleared on leaving alloy mode"
        );
    }

    #[rstest::rstest]
    fn confirm_provider_with_multiple_selected_creates_alloy() {
        // Given a picker with two available entries, both selected.
        let mut state = AppState::default();
        let origin = ChatSessionState::new();
        state.session.insert(origin);
        state
            .session
            .set_active(state.session.active_session_id().clone());

        let entries = vec![
            crate::protocol::PickerEntry {
                provider_id: "ollama/llama3".to_owned(),
                name: "ollama".to_owned(),
                provider_name: "ollama".to_owned(),
                backend: "ollama".to_owned(),
                model: "llama3".to_owned(),
                search_text: "llama3".to_owned(),
                is_alias: false,
                alias_target: None,
                is_available: true,
                is_remote: false,
                is_active: false,
                selected: true, // Selected!
                theme: crate::feat::theme::default_theme(),
            },
            crate::protocol::PickerEntry {
                provider_id: "openrouter/gpt-4".to_owned(),
                name: "openrouter".to_owned(),
                provider_name: "openrouter".to_owned(),
                backend: "openrouter".to_owned(),
                model: "gpt-4".to_owned(),
                search_text: "gpt-4".to_owned(),
                is_alias: false,
                alias_target: None,
                is_available: true,
                is_remote: false,
                is_active: false,
                selected: true, // Selected!
                theme: crate::feat::theme::default_theme(),
            },
        ];
        state.provider.provider_picker.set_items(entries);
        state.provider.alloy_mode = true;

        let result = confirm_provider(&mut state);

        // Then a ProviderSwitch message is emitted for alloy.
        assert!(!result.message_names.is_empty());
        assert!(
            result
                .message_names
                .iter()
                .any(|n| n.contains("ProviderSwitch")),
            "messages should contain ProviderSwitch: {:?}",
            result.message_names
        );
    }

    #[rstest::rstest]
    fn confirm_persona_sets_correct_persona() {
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
        assert!(!result.message_names.is_empty());
    }

    #[rstest::rstest]
    fn confirm_reasoning_effort_sets_session_override() {
        // If the session override were never set, the profile would stay None.
        use crate::feat::reasoning::{ReasoningEffort, ReasoningEffortEntry};

        let mut state = AppState::default();
        let origin = ChatSessionState::new();
        state.session.insert(origin);
        state
            .session
            .set_active(state.session.active_session_id().clone());

        // Populate the picker with a single entry; select it.
        let entry = ReasoningEffortEntry {
            effort: ReasoningEffort::High,
            name: "high".to_owned(),
            description: "High effort".to_owned(),
            is_active: false,
            theme: crate::feat::theme::default_theme(),
        };
        state
            .frontend
            .reasoning_effort_picker_mut()
            .set_items(vec![entry]);
        state.frontend.reasoning_effort_picker_mut().move_down(1);

        // When confirming.
        let _ = confirm_reasoning_effort(&mut state);

        // Then the active session's override is set to High.
        assert_eq!(
            state.active_session().profile().reasoning_effort,
            Some(ReasoningEffort::High),
            "confirm should set the session reasoning_effort override"
        );
    }

    #[rstest::rstest]
    fn confirm_reasoning_effort_in_session_a_does_not_leak_into_session_b() {
        // Regression: changing effort in one session used to leak into every other
        // override-free session because the live global was consulted at request time.
        // Now each session owns its own value; the global seeds new sessions only.
        use crate::feat::reasoning::{ReasoningEffort, ReasoningEffortEntry, resolve_effort};

        let mut state = AppState::default();

        // Session B: seeded with High (its own, frozen value).
        let mut b = ChatSessionState::new();
        b.profile_mut().reasoning_effort = Some(ReasoningEffort::High);
        let b_id = b.session_id().clone();
        state.session.insert(b);

        // Session A (active): seeded with High, then changed to Xhigh via the picker.
        let mut a = ChatSessionState::new();
        a.profile_mut().reasoning_effort = Some(ReasoningEffort::High);
        state.session.insert(a);
        state
            .session
            .set_active(state.session.active_session_id().clone());

        let entry = ReasoningEffortEntry {
            effort: ReasoningEffort::Xhigh,
            name: "xhigh".to_owned(),
            description: "Extra high effort".to_owned(),
            is_active: false,
            theme: crate::feat::theme::default_theme(),
        };
        state
            .frontend
            .reasoning_effort_picker_mut()
            .set_items(vec![entry]);
        state.frontend.reasoning_effort_picker_mut().move_down(1);

        // When confirming the effort change in session A.
        let result = confirm_reasoning_effort(&mut state);

        // Then session A's own effort is Xhigh.
        assert_eq!(
            state.active_session().profile().reasoning_effort,
            Some(ReasoningEffort::Xhigh),
            "session A should have the confirmed effort"
        );
        // And the global default was advanced to Xhigh (so future sessions inherit it).
        assert!(
            result
                .message_names
                .iter()
                .any(|n| n.ends_with("UpdatePreferences")),
            "confirm should still seed the global for future sessions"
        );
        // But session B's resolved effort is unchanged — the global no longer leaks.
        assert_eq!(
            resolve_effort(state.session.get(&b_id).unwrap().profile().reasoning_effort),
            Some(ReasoningEffort::High),
            "session B's own effort must be unaffected by session A's change"
        );
    }

    #[rstest::rstest]
    fn confirm_reasoning_effort_pops_picker_scope() {
        // If the scope were never popped, the picker would remain open.
        use crate::common::app_state::FocusScope;
        use crate::feat::reasoning::{ReasoningEffort, ReasoningEffortEntry};

        let mut state = AppState::default();
        let origin = ChatSessionState::new();
        state.session.insert(origin);
        state
            .session
            .set_active(state.session.active_session_id().clone());
        let entry = ReasoningEffortEntry {
            effort: ReasoningEffort::Medium,
            name: "medium".to_owned(),
            description: "Medium effort".to_owned(),
            is_active: false,
            theme: crate::feat::theme::default_theme(),
        };
        state
            .frontend
            .reasoning_effort_picker_mut()
            .set_items(vec![entry]);
        state.frontend.reasoning_effort_picker_mut().move_down(1);
        state.frontend.scope_stack.push(FocusScope::Picker {
            kind: PickerKind::ReasoningEffort,
        });

        // When confirming.
        let _ = confirm_reasoning_effort(&mut state);

        // Then the picker scope has been popped (no ReasoningEffort scope remains).
        let still_open = state
            .frontend
            .scope_stack
            .picker_kind()
            .is_some_and(|k| *k == PickerKind::ReasoningEffort);
        assert!(!still_open, "picker scope should be popped after confirm");
    }

    #[rstest::rstest]
    fn confirm_reasoning_effort_emits_mark_session_interacted() {
        // If the persist message were never emitted, the profile change would
        // only be saved on a later (unrelated) event.
        use crate::feat::reasoning::{ReasoningEffort, ReasoningEffortEntry};

        let mut state = AppState::default();
        let origin = ChatSessionState::new();
        state.session.insert(origin);
        state
            .session
            .set_active(state.session.active_session_id().clone());
        let entry = ReasoningEffortEntry {
            effort: ReasoningEffort::Low,
            name: "low".to_owned(),
            description: "Low effort".to_owned(),
            is_active: false,
            theme: crate::feat::theme::default_theme(),
        };
        state
            .frontend
            .reasoning_effort_picker_mut()
            .set_items(vec![entry]);
        state.frontend.reasoning_effort_picker_mut().move_down(1);

        // When confirming.
        let result = confirm_reasoning_effort(&mut state);

        // Then a MarkSessionInteracted message is emitted.
        assert!(
            result
                .message_names
                .iter()
                .any(|n| n.ends_with("MarkSessionInteracted")),
            "confirm should emit MarkSessionInteracted to persist the change"
        );
    }

    #[rstest::rstest]
    fn confirm_reasoning_effort_emits_update_preferences() {
        // If the global default write were never emitted, new sessions would
        // not inherit the chosen effort.
        use crate::feat::reasoning::{ReasoningEffort, ReasoningEffortEntry};

        let mut state = AppState::default();
        let origin = ChatSessionState::new();
        state.session.insert(origin);
        state
            .session
            .set_active(state.session.active_session_id().clone());
        let entry = ReasoningEffortEntry {
            effort: ReasoningEffort::Max,
            name: "max".to_owned(),
            description: "Maximum effort".to_owned(),
            is_active: false,
            theme: crate::feat::theme::default_theme(),
        };
        state
            .frontend
            .reasoning_effort_picker_mut()
            .set_items(vec![entry]);
        state.frontend.reasoning_effort_picker_mut().move_down(1);

        // When confirming.
        let result = confirm_reasoning_effort(&mut state);

        // Then an UpdatePreferences message is emitted (global default write).
        assert!(
            result
                .message_names
                .iter()
                .any(|n| n.ends_with("UpdatePreferences")),
            "confirm should emit UpdatePreferences to persist the global default"
        );
    }

    #[rstest::rstest]
    fn confirm_persona_emits_mark_session_interacted() {
        // added to confirm_persona.
        // If the persist message were never emitted, a pick-then-quit would lose
        // the persona change.
        use crate::feat::persona::PersonaEntry;

        let mut state = AppState::default();
        let origin = ChatSessionState::new();
        state.session.insert(origin);
        state
            .session
            .set_active(state.session.active_session_id().clone());
        state.context.personas = vec![crate::feat::persona::Persona {
            name: "coder".to_owned(),
            description: String::new(),
            body: "You are a coder.".to_owned(),
            file_path: PathBuf::new(),
        }];
        let entry = PersonaEntry {
            name: "coder".to_owned(),
            description: String::new(),
            is_active: false,
            theme: crate::feat::theme::default_theme(),
        };
        state.frontend.persona_picker_mut().set_items(vec![entry]);
        state.frontend.persona_picker_mut().move_down(1);

        // When confirming.
        let result = confirm_persona(&mut state);

        // Then a MarkSessionInteracted message is emitted.
        assert!(
            result
                .message_names
                .iter()
                .any(|n| n.ends_with("MarkSessionInteracted")),
            "confirm_persona should emit MarkSessionInteracted to persist"
        );
    }

    #[rstest::rstest]
    fn confirm_session_lifecycle_finds_correct_lifecycle_for_args() {
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

        // Then an AttachPlugin command is emitted.
        assert_eq!(result.message_names.len(), 1);
        assert!(result.message_names[0].contains("AttachPlugin"));
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
                .message_names
                .iter()
                .any(|n| n.contains("ScanSkills")),
            "expected ScanSkills command"
        );
        assert!(
            result
                .message_names
                .iter()
                .any(|n| n.contains("RescanPromptTemplates")),
            "expected RescanPromptTemplates command"
        );
        assert!(
            result
                .message_names
                .iter()
                .any(|n| n.contains("ScanContextFiles")),
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
        assert!(result.message_names.is_empty());
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
            .postpone_task(&to_postpone, TaskPosition::After(task_cancel))
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
        assert!(result.message_names.is_empty(), "no commands");
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

    fn setup_state_with_plugin_tools() -> AppState {
        let mut state = AppState::default();
        let origin = ChatSessionState::new();
        state.session.insert(origin);
        state
            .session
            .set_active(state.session.active_session_id().clone());

        // Simulate plugin tools having been registered via ToolsRegistered event.
        state.context.global_tool_definitions.insert(
            "judgment_passed".to_owned(),
            crate::protocol::ToolDefinition {
                name: "judgment_passed".to_owned(),
                description: "Call when response passes".to_owned(),
                parameters: serde_json::json!({"type": "object", "properties": {}}),
                prompt_snippet: None,
                prompt_guidelines: vec![],
                server_tool_type: None,
            },
        );
        state.context.global_tool_definitions.insert(
            "judgment_failed".to_owned(),
            crate::protocol::ToolDefinition {
                name: "judgment_failed".to_owned(),
                description: "Call when response fails".to_owned(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "message": { "type": "string", "description": "Why it failed" }
                    },
                    "required": ["message"]
                }),
                prompt_snippet: None,
                prompt_guidelines: vec![],
                server_tool_type: None,
            },
        );

        state
    }

    #[rstest::rstest]
    fn load_tool_picker_entries_includes_plugin_tools() {
        // Given state with plugin tool definitions in context.
        let mut state = setup_state_with_plugin_tools();

        // When loading tool picker entries.
        load_tool_picker_entries(&mut state);

        // Then plugin tools appear alongside any builtins.
        let items = state.frontend.tool_picker().items();
        let names: Vec<&str> = items.iter().map(|e| e.name.as_str()).collect();
        assert!(
            names.contains(&"judgment_passed"),
            "plugin tool 'judgment_passed' should be in picker, got: {names:?}"
        );
        assert!(
            names.contains(&"judgment_failed"),
            "plugin tool 'judgment_failed' should be in picker, got: {names:?}"
        );
    }

    #[rstest::rstest]
    fn load_tool_picker_entries_marks_plugin_tools_enabled_by_default() {
        // Given state with plugin tool definitions.
        let mut state = setup_state_with_plugin_tools();

        // When loading tool picker entries.
        load_tool_picker_entries(&mut state);

        // Then plugin tools are enabled (not in disabled_tools set).
        let items = state.frontend.tool_picker().items();
        let passed = items
            .iter()
            .find(|e| e.name == "judgment_passed")
            .expect("entry");
        assert!(
            passed.enabled,
            "judgment_passed should be enabled by default"
        );
        let failed = items
            .iter()
            .find(|e| e.name == "judgment_failed")
            .expect("entry");
        assert!(
            failed.enabled,
            "judgment_failed should be enabled by default"
        );
    }

    #[rstest::rstest]
    fn load_tool_picker_entries_disables_plugin_tool_when_in_disabled_set() {
        // Given state with 'judgment_passed' in the disabled_tools set.
        let mut state = setup_state_with_plugin_tools();
        state
            .active_session_mut()
            .set_disabled_tools(std::collections::HashSet::from([
                "judgment_passed".to_owned()
            ]));

        // When loading tool picker entries.
        load_tool_picker_entries(&mut state);

        // Then 'judgment_passed' is disabled but 'judgment_failed' is still enabled.
        let items = state.frontend.tool_picker().items();
        let passed = items
            .iter()
            .find(|e| e.name == "judgment_passed")
            .expect("entry");
        assert!(!passed.enabled, "judgment_passed should be disabled");
        let failed = items
            .iter()
            .find(|e| e.name == "judgment_failed")
            .expect("entry");
        assert!(failed.enabled, "judgment_failed should still be enabled");
    }

    /// Builds a state with the provider picker open (Provider scope), `n` available
    /// single-model entries `model-0..model-n`, and the first entry highlighted.
    fn state_with_provider_picker(n: usize) -> AppState {
        let mut state = AppState::default();
        let origin = ChatSessionState::new();
        state.session.insert(origin);
        state
            .session
            .set_active(state.session.active_session_id().clone());
        state.frontend.scope_stack.push(FocusScope::Picker {
            kind: PickerKind::Provider,
        });
        let entries: Vec<crate::protocol::PickerEntry> = (0..n)
            .map(|i| crate::protocol::PickerEntry {
                provider_id: format!("prov/model-{i}"),
                name: "prov".to_owned(),
                provider_name: "prov".to_owned(),
                backend: "openrouter".to_owned(),
                model: format!("model-{i}"),
                search_text: format!("model-{i}"),
                is_alias: false,
                alias_target: None,
                is_available: true,
                is_remote: false,
                is_active: false,
                selected: false,
                theme: crate::feat::theme::default_theme(),
            })
            .collect();
        state.provider.provider_picker.set_items(entries);
        state.provider.provider_picker.move_down(1); // highlight first entry
        state
    }

    #[rstest::rstest]
    fn single_mode_resolve_returns_highlight_ignoring_checks() {
        // Given single mode with a stale check on model-0 and model-1 highlighted.
        let mut state = state_with_provider_picker(2);
        state.provider.alloy_mode = false;
        // Stale check on model-0 that single mode must ignore.
        state.provider.provider_picker.with_selected_mut(|e| {
            e.selected = true;
        });
        state.provider.provider_picker.move_down(1); // highlight model-1

        // When resolving the selection for the highlighted entry.
        let selection = resolve_provider_selection(&state.provider, "prov/model-1".to_owned());

        // Then it is Single of the highlighted entry, not the checked one.
        assert_eq!(selection, ModelSelection::Single("prov/model-1".to_owned()));
    }

    #[rstest::rstest]
    fn single_mode_confirm_rejects_unavailable_highlight() {
        // Given single mode where the highlighted entry is unavailable.
        let mut state = state_with_provider_picker(1);
        state.provider.alloy_mode = false;
        state.provider.provider_picker.with_selected_mut(|e| {
            e.is_available = false;
        });

        // When confirming.
        let result = confirm_provider(&mut state);

        // Then no ProviderSwitch is emitted.
        assert!(
            !result
                .message_names
                .iter()
                .any(|n| n.contains("ProviderSwitch")),
            "unavailable highlight should be rejected"
        );
    }

    #[rstest::rstest]
    fn single_mode_tab_is_noop() {
        // Given single mode.
        let mut state = state_with_provider_picker(2);
        state.provider.alloy_mode = false;

        // When toggling a model.
        handle_model_toggle(&mut state);

        // Then no entry became selected.
        let any_selected = state
            .provider
            .provider_picker
            .items()
            .iter()
            .any(|e| e.selected);
        assert!(!any_selected, "single-mode TAB must not check anything");
    }

    #[rstest::rstest]
    fn alloy_mode_resolve_includes_highlight_and_checks() {
        // Given alloy mode with model-0 checked and model-2 highlighted.
        let mut state = state_with_provider_picker(3);
        state.provider.alloy_mode = true;
        // Check model-0.
        state.provider.provider_picker.move_up(PICKER_MAX_VISIBLE);
        state.provider.provider_picker.with_selected_mut(|e| {
            e.selected = true;
        });
        // Move to model-2 (down twice from model-0).
        state.provider.provider_picker.move_down(PICKER_MAX_VISIBLE);
        state.provider.provider_picker.move_down(PICKER_MAX_VISIBLE);

        // When resolving the selection for the highlighted entry.
        let selection = resolve_provider_selection(&state.provider, "prov/model-2".to_owned());

        // Then it is an Alloy containing both model-0 and model-2.
        match selection {
            ModelSelection::Alloy { models, .. } => {
                assert_eq!(models.len(), 2);
                assert!(models.contains(&"prov/model-0".to_owned()));
                assert!(models.contains(&"prov/model-2".to_owned()));
            }
            other => panic!("expected Alloy, got {other:?}"),
        }
    }

    #[rstest::rstest]
    fn alloy_mode_resolve_dedups_already_checked_highlight() {
        // Given alloy mode with the highlighted entry (model-1) already checked.
        let mut state = state_with_provider_picker(2);
        state.provider.alloy_mode = true;
        state.provider.provider_picker.with_selected_mut(|e| {
            e.selected = true;
        });

        // When resolving (highlight is already checked).
        let selection = resolve_provider_selection(&state.provider, "prov/model-1".to_owned());

        // Then it collapses to Single (one model, no duplication).
        assert_eq!(
            selection,
            ModelSelection::Single("prov/model-1".to_owned()),
            "already-checked highlight must not duplicate; 1-model set collapses to Single"
        );
    }

    #[rstest::rstest]
    fn alloy_mode_resolve_one_model_collapses_to_single() {
        // Given alloy mode with nothing checked and the highlighted entry (model-1).
        let mut state = state_with_provider_picker(2);
        state.provider.alloy_mode = true;

        // When resolving the selection for the highlighted entry.
        let selection = resolve_provider_selection(&state.provider, "prov/model-1".to_owned());

        // Then a Single selection is returned (1-model alloy collapses).
        assert_eq!(selection, ModelSelection::Single("prov/model-1".to_owned()));
    }

    /// Builds an AppState with a project picker open, the active session's CWD
    /// set to a distinct value, and `n` curated project entries loaded.
    fn state_with_project_picker(paths: &[&str]) -> AppState {
        let mut state = AppState::default();
        let origin = ChatSessionState::new();
        state.session.insert(origin);
        state
            .session
            .set_active(state.session.active_session_id().clone());
        state
            .active_session_mut()
            .set_cwd(std::path::PathBuf::from("/tmp/active-session-cwd"));
        state.frontend.scope_stack.push(FocusScope::Picker {
            kind: PickerKind::Project,
        });
        let projects: Vec<crate::feat::project::ProjectConfig> = paths
            .iter()
            .map(|p| crate::feat::project::ProjectConfig {
                path: std::path::PathBuf::from(p),
            })
            .collect();
        state.frontend.preferences.projects = projects;
        load_project_picker_entries(&mut state);
        // index 0 is selected by default after set_items + reset.
        state
    }

    #[rstest::rstest]
    fn confirm_project_creates_new_session_at_chosen_dir() {
        // Given a project picker whose highlighted entry is /tmp/project-a.
        let mut state = state_with_project_picker(&["/tmp/project-a", "/tmp/project-b"]);

        // When confirming the highlighted project (Enter).
        let result = confirm_project(&mut state);

        // Then a new session was created (a message was emitted to drive it).
        assert!(!result.message_names.is_empty());
        // And the new active session's CWD is the chosen project dir, not the
        // previously active session's CWD.
        assert_eq!(
            state.active_session().cwd(),
            std::path::Path::new("/tmp/project-a"),
        );
        // And the pending override was consumed.
        assert!(state.frontend.pending_session_cwd.is_none());
    }

    #[rstest::rstest]
    fn confirm_project_leaves_previous_session_cwd_unchanged() {
        // Given a project picker with an existing active session.
        let mut state = state_with_project_picker(&["/tmp/project-a"]);
        let prev_id = state.session.active_session_id().clone();

        // When confirming the highlighted project.
        let _result = confirm_project(&mut state);

        // Then the previous session (now backgrounded) keeps its original CWD.
        let prev = state
            .session
            .get(&prev_id)
            .expect("previous session still exists");
        assert_eq!(prev.cwd(), std::path::Path::new("/tmp/active-session-cwd"));
    }

    #[rstest::rstest]
    fn confirm_project_with_lifecycle_sets_override_then_opens_lifecycle_picker() {
        // Given a project picker whose highlighted entry is /tmp/project-a.
        let mut state = state_with_project_picker(&["/tmp/project-a"]);

        // When confirming with lifecycle (<c-enter>).
        let _result = handle_project_lifecycle_confirm(&mut state);

        // Then the project scope was popped and the lifecycle picker opened.
        assert!(matches!(
            state.frontend.scope_stack.current(),
            FocusScope::Picker {
                kind: PickerKind::SessionLifecycle
            }
        ));
        // And the chosen dir is stashed as the pending override, awaiting the
        // lifecycle/args confirm chain.
        assert_eq!(
            state.frontend.pending_session_cwd.as_deref(),
            Some(std::path::Path::new("/tmp/project-a")),
        );
    }

    #[rstest::rstest]
    fn project_remove_highlighted_deletes_highlighted_entry() {
        // Given a project picker with two entries and the first highlighted.
        let mut state = state_with_project_picker(&["/tmp/project-a", "/tmp/project-b"]);

        // When removing the highlighted entry (d).
        let result = handle_project_remove_highlighted(&mut state);

        // Then the highlighted entry is removed from preferences.projects.
        let paths: Vec<_> = state
            .frontend
            .preferences
            .projects
            .iter()
            .map(|p| p.path.clone())
            .collect();
        assert_eq!(paths, vec![std::path::PathBuf::from("/tmp/project-b")]);
        // And an UpdatePreferences(RemoveProject) message was emitted.
        assert!(!result.message_names.is_empty());
        // And the picker now shows one entry.
        assert_eq!(state.frontend.project_picker().items().len(), 1);
    }

    fn setup_state_with_web_search_tool(model: &str) -> AppState {
        let mut state = AppState::default();
        let origin = ChatSessionState::new();
        state.session.insert(origin);
        state
            .session
            .set_active(state.session.active_session_id().clone());
        state
            .active_session_mut()
            .set_model(ModelSelection::Single(model.to_owned()));

        state.context.global_tool_definitions.insert(
            "openrouter:web_search".to_owned(),
            crate::protocol::ToolDefinition {
                name: "openrouter:web_search".to_owned(),
                description: "Search the web".to_owned(),
                parameters: serde_json::json!({}),
                prompt_snippet: None,
                prompt_guidelines: vec![],
                server_tool_type: Some(jinn_provider::ServerToolType::OpenrouterWebSearch),
            },
        );
        state
    }

    #[rstest::rstest]
    fn load_tool_picker_entries_hides_web_search_for_non_openrouter_model() {
        // Given state on a non-openrouter model with a web_search tool registered.
        let mut state = setup_state_with_web_search_tool("zai/glm-4.6");

        // When loading tool picker entries.
        load_tool_picker_entries(&mut state);

        // Then the web_search tool is NOT offered (it can't run on this provider).
        let names: Vec<&str> = state
            .frontend
            .tool_picker()
            .items()
            .iter()
            .map(|e| e.name.as_str())
            .collect();
        assert!(
            !names.contains(&"openrouter:web_search"),
            "web_search should be hidden for non-openrouter model, got: {names:?}"
        );
    }

    #[rstest::rstest]
    fn load_tool_picker_entries_shows_web_search_for_openrouter_model() {
        // Given state on an openrouter model with a web_search tool registered.
        let mut state = setup_state_with_web_search_tool("openrouter/openai/gpt-oss-120b");

        // When loading tool picker entries.
        load_tool_picker_entries(&mut state);

        // Then the web_search tool IS offered.
        let names: Vec<&str> = state
            .frontend
            .tool_picker()
            .items()
            .iter()
            .map(|e| e.name.as_str())
            .collect();
        assert!(
            names.contains(&"openrouter:web_search"),
            "web_search should be visible for openrouter model, got: {names:?}"
        );
    }
}
