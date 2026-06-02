//! Picker intent handlers - navigation, filtering, confirmation, and scope toggling.
//!
//! Handles all picker intents: open, insert char, backspace, confirm, move up/down,
//! cursor movement, and keymap scope filter toggle. The `handle_picker_confirm`
//! function returns `(IntentResult, Option<Intent>)` to allow the caller
//! (`jinn-intent`) to re-dispatch keymap intents without creating a circular
//! dependency.

use crate::common::app_state::AppState;
use crate::common::app_state::FocusScope;
use crate::feat::context::protocol::command::LoadPersonaPickerEntries;
use crate::feat::preferences_actor::protocol::command::{PreferenceUpdate, UpdatePreferences};
use crate::feat::provider::protocol::command::{LoadProviderPickerEntries, ProviderSwitch};
#[cfg(test)]
use crate::feat::session::chat_session::ChatSessionState;
use crate::feat::session::protocol::load_session_picker_entries::LoadSessionPickerEntries;
use crate::feat::session::protocol::session_load_requested::SessionLoadRequested;

use crate::feat::ui::picker_states::PickerExt;
use crate::protocol::{Command, Intent, IntentResult, PickerKind};

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
        PickerKind::Workflow => {
            state.frontend.workflow_picker_mut().reset();
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
        PickerKind::Theme | PickerKind::Tool | PickerKind::Skill => IntentResult::empty(),
        PickerKind::SessionLifecycle => {
            // Populate from user preferences + implicit blank lifecycle.
            load_lifecycle_picker_entries(state);
            IntentResult::empty()
        }
        PickerKind::Workflow => {
            // Entries will be populated by WorkflowActor via LoadWorkflowPickerEntries command.
            IntentResult::with_commands(vec![Command::LoadWorkflowPickerEntries(
                crate::feat::workflow::protocol::command::LoadWorkflowPickerEntries,
            )])
        }

        PickerKind::CompactionModel => {
            IntentResult::with_commands(vec![Command::LoadCompactionModelPickerEntries(
                crate::feat::provider::protocol::command::LoadCompactionModelPickerEntries,
            )])
        }
    }
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
        state.frontend.skill_preview_scroll().saturating_sub(page_size));
    IntentResult::empty()
}

/// Scrolls the preview pane down by one page.
pub fn handle_preview_scroll_down(state: &mut AppState) -> IntentResult {
    let page_size = preview_page_size(state);
    state.frontend.set_skill_preview_scroll(
        state.frontend.skill_preview_scroll().saturating_add(page_size));
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
        Some(PickerKind::Workflow) => (confirm_workflow(state), None),

        Some(PickerKind::CompactionModel) | None => (IntentResult::empty(), None),
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
            provider_id: provider_id.clone(),
        }),
        Command::UpdatePreferences(UpdatePreferences {
            updates: vec![PreferenceUpdate::SetLastModel(Some(provider_id))],
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

    IntentResult::with_commands(vec![Command::UpdatePreferences(UpdatePreferences {
        updates: vec![PreferenceUpdate::SetPersona(Some(persona_name))],
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

    IntentResult::with_commands(vec![Command::UpdatePreferences(UpdatePreferences {
        updates: vec![PreferenceUpdate::SetTheme(Some(theme_name))],
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

    state.frontend.session_lifecycle_picker_mut().set_items(entries);
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
            input: String::new(),
            cursor_pos: 0,
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

/// Confirms the selected workflow, starts it, and switches to the Workflow tab.
fn confirm_workflow(state: &mut AppState) -> IntentResult {
    let Some(entry) = state.frontend.workflow_picker().selected_item() else {
        return IntentResult::empty();
    };
    let name = entry.name.clone();
    let workflow_id = crate::feat::workflow::WorkflowId::new();

    // Pop picker overlay.
    state.frontend.scope_stack.pop();



    state
        .frontend
        .scope_stack
        .swap_base(crate::common::app_state::FocusScope::Workflow);
    let config = crate::feat::workflow::attached_workflow::WorkflowConfig::Custom(
        serde_json::json!({"name": name}),
    );
    let session_id = state.session.active_session_id().clone();
    IntentResult::with_commands(vec![Command::InitWorkflow(
        crate::feat::workflow::protocol::command::InitWorkflow {
            name,
            workflow_id,
            session_id,
            config,
            trigger: crate::feat::workflow::attached_workflow::WorkflowTrigger::Manual,
        },
    )])
}

/// Populates the tool picker entries from the global tool definitions.
///
/// Marks each entry as enabled/disabled based on the session's `disabled_tools` set.

fn load_tool_picker_entries(state: &mut AppState) {
    use crate::feat::tools_actor::tool_entry::ToolEntry;

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
    state.frontend.tool_picker_mut().move_down(PICKER_MAX_VISIBLE);
    IntentResult::empty()
}

/// Populates the skill picker entries from discovered skills.
///
/// Marks each entry as enabled/disabled based on the session's `disabled_skills` set.
fn load_skill_picker_entries(state: &mut AppState) {
    use crate::feat::skills::skill_entry::SkillEntry;

    let disabled = state.active_session().disabled_skills();
    let theme = state.frontend.theme.clone();

    let mut entries: Vec<SkillEntry> = state
        .context
        .skills
        .iter()
        .map(|skill| {
            let name = skill.name.clone();
            let description = skill.description.clone();
            SkillEntry {
                search_text: format!("{name} {description}"),
                name,
                description,
                body: skill.body.clone(),
                enabled: !disabled.contains(&skill.name),
                theme: theme.clone(),
            }
        })
        .collect();

    entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    state.frontend.skill_picker_mut().set_items(entries);
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
    state.frontend.skill_picker_mut().with_selected_mut(|entry| {
        entry.enabled = !entry.enabled;
    });
    state.frontend.skill_picker_mut().move_down(PICKER_MAX_VISIBLE);
    IntentResult::empty()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use super::*;
    use std::path::PathBuf;

    // --- A-Tier: Kill mutants for picker confirm and validation ---

    #[rstest::rstest]
    fn confirm_provider_rejects_unavailable() {
        // Kills: delete ! in confirm_provider.
        // If the ! were deleted, unavailable providers could be confirmed.
        let mut state = AppState::default();
        let origin = ChatSessionState::new();
        state.session.insert(origin);
        state.session.set_active(
            state.session.active_session_id().clone(),
        );

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
        state.session.set_active(
            state.session.active_session_id().clone(),
        );

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
        state.session.set_active(
            state.session.active_session_id().clone(),
        );

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
            state.context.active_persona.as_ref().map(|p| p.name.as_str()),
            Some("writer"),
            "confirm_persona should set the correct persona"
        );
        assert!(!result.commands.is_empty());
    }

    // --- A-Tier: Kill mutant for == with != in confirm_session_lifecycle ---

    #[rstest::rstest]
    fn confirm_session_lifecycle_finds_correct_lifecycle_for_args() {
        // Kills: replace == with != in confirm_session_lifecycle.
        // If the match were inverted, find() would locate the WRONG lifecycle,
        // producing the wrong template_display in the arg_input state.
        let mut state = AppState::default();
        let origin = ChatSessionState::new();
        state.session.insert(origin);
        state.session.set_active(
            state.session.active_session_id().clone(),
        );

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

    // --- B-Tier: Kill mutant for replace load_lifecycle_picker_entries with () ---

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
        assert_eq!(items.len(), 2, "should have blank + 1 lifecycle = 2 entries");
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
        state.session.set_active(
            state.session.active_session_id().clone(),
        );

        state.context.skills = vec![
            Skill {
                name: "phased-task-loop".to_owned(),
                description: "Structured phased implementation workflow".to_owned(),
                body: String::new(),
                file_path: PathBuf::from("/tmp/skills/phased-task-loop/SKILL.md"),
                base_dir: PathBuf::from("/tmp/skills/phased-task-loop"),
            },
            Skill {
                name: "web-coder".to_owned(),
                description: "Expert web development".to_owned(),
                body: String::new(),
                file_path: PathBuf::from("/tmp/skills/web-coder/SKILL.md"),
                base_dir: PathBuf::from("/tmp/skills/web-coder"),
            },
        ];

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
    fn load_skill_picker_entries_marks_disabled() {
        // Given state with "web-coder" disabled.
        let mut state = setup_state_with_skills();
        state.active_session_mut().set_disabled_skills(
            std::collections::HashSet::from(["web-coder".to_owned()]),
        );

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
        assert_eq!(disabled, std::collections::HashSet::from(["web-coder".to_owned()]));
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
}
