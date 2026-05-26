//! Picker intent handlers — navigation, filtering, confirmation, and scope toggling.
//!
//! Handles all picker intents: open, insert char, backspace, confirm, move up/down,
//! cursor movement, and keymap scope filter toggle. The `handle_picker_confirm`
//! function returns `(IntentResult, Option<Intent>)` to allow the caller
//! (`nullslop-intent`) to re-dispatch keymap intents without creating a circular
//! dependency.

use crate::common::app_state::AppState;
use crate::common::app_state::FocusScope;
use crate::feat::context::protocol::command::LoadPersonaPickerEntries;
use crate::feat::preferences_actor::protocol::command::{PreferenceUpdate, UpdatePreferences};
use crate::feat::provider::protocol::command::{LoadProviderPickerEntries, ProviderSwitch};
use crate::feat::session::protocol::load_session_picker_entries::LoadSessionPickerEntries;
use crate::feat::session::protocol::session_load_requested::SessionLoadRequested;
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
            state.frontend.session_picker.reset();
        }
        PickerKind::Persona => {
            state.frontend.persona_picker.reset();
        }
        PickerKind::Theme => {
            state.frontend.theme_picker.reset();
            // Save current theme so ESC can restore it.
            state.frontend.theme_preview_original = Some(state.frontend.theme.clone());
            // Load discovered themes as entries.
            load_theme_picker_entries(state);
        }
        PickerKind::SessionLifecycle => {
            state.frontend.session_lifecycle_picker.reset();
        }
        PickerKind::Workflow => {
            state.frontend.workflow_picker.reset();
        }
        PickerKind::Judge => {
            state.frontend.judge_picker.reset();
        }
        PickerKind::CompactionModel => {
            state.frontend.compaction_model_picker.reset();
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
        PickerKind::Theme => IntentResult::empty(),
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
        PickerKind::Judge => {
            // Populate from scanned judge definitions (already in state.context.judges).
            load_judge_picker_entries(state);
            IntentResult::empty()
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

    state.frontend.theme_picker.set_items(entries);
}

/// Previews the selected theme in real-time when the Theme picker is active.
fn preview_theme_if_active(state: &mut AppState) {
    if state.frontend.scope_stack.picker_kind() != Some(&PickerKind::Theme) {
        return;
    }
    if let Some(entry) = state.frontend.theme_picker.selected_item() {
        state.frontend.theme = entry.theme.clone();
    }
}

/// Inserts a character into the active picker's filter.
pub fn handle_insert_char(state: &mut AppState, ch: char) -> IntentResult {
    validator::validate_picker_insert_char(state, ch);
    if let Some(picker) = state.active_picker_ops() {
        picker.insert_char(ch);
    }
    IntentResult::empty()
}

/// Handles `PasteText` in picker scope — bulk inserts pasted text into the filter.
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
        Some(PickerKind::Judge) => (confirm_judge(state), None),
        Some(PickerKind::CompactionModel) | None => (IntentResult::empty(), None),
    }
}

/// Moves the selection up in the active picker.
pub fn handle_move_up(state: &mut AppState) -> IntentResult {
    validator::validate_picker_move_up(state);
    if let Some(picker) = state.active_picker_ops() {
        picker.move_up(PICKER_MAX_VISIBLE);
    }
    preview_theme_if_active(state);
    IntentResult::empty()
}

/// Moves the selection down in the active picker.
pub fn handle_move_down(state: &mut AppState) -> IntentResult {
    validator::validate_picker_move_down(state);
    if let Some(picker) = state.active_picker_ops() {
        picker.move_down(PICKER_MAX_VISIBLE);
    }
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
    let Some(entry) = state.frontend.persona_picker.selected_item() else {
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
    let Some(entry) = state.frontend.theme_picker.selected_item() else {
        return IntentResult::empty();
    };
    let theme_name = entry.name.clone();

    // Theme is already previewed (set on move). Just persist.
    state.frontend.theme_preview_original = None;
    state.frontend.scope_stack.pop();

    IntentResult::with_commands(vec![Command::UpdatePreferences(UpdatePreferences {
        updates: vec![PreferenceUpdate::SetTheme(Some(theme_name))],
    })])
}

/// Confirms the selected session and dispatches a switch command.
fn confirm_session(state: &mut AppState) -> IntentResult {
    let Some(entry) = state.frontend.session_picker.selected_item() else {
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
/// as the first entry, followed by all lifecycles defined in `nullslop.toml`.
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

    state.frontend.session_lifecycle_picker.set_items(entries);
}

/// Confirms the selected session lifecycle.
///
/// If the lifecycle has args, the arg input popup would open (Phase 6).
/// For now, directly triggers setup with empty args (lifecycles without args)
/// or with empty args as a placeholder.
fn confirm_session_lifecycle(state: &mut AppState) -> IntentResult {
    let Some(entry) = state.frontend.session_lifecycle_picker.selected_item() else {
        return IntentResult::empty();
    };

    let lifecycle_name = entry.name.clone();
    state.frontend.scope_stack.pop();

    if entry.has_args {
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

    // No args — proceed directly.
    crate::feat::session_lifecycle::intent::handle_session_lifecycle_setup(
        state,
        &lifecycle_name,
        &[],
    )
}

/// Confirms the selected workflow, starts it, and switches to the Workflow tab.
fn confirm_workflow(state: &mut AppState) -> IntentResult {
    let Some(entry) = state.frontend.workflow_picker.selected_item() else {
        return IntentResult::empty();
    };
    let name = entry.name.clone();
    let workflow_id = crate::feat::workflow::WorkflowId::new();

    // Pop picker overlay.
    state.frontend.scope_stack.pop();

    // Switch to workflow tab.
    state.frontend.active_tab = crate::protocol::tab::ActiveTab::Workflow;
    state
        .frontend
        .scope_stack
        .swap_base(crate::common::app_state::FocusScope::Workflow);

    IntentResult::with_commands(vec![Command::InitWorkflow(
        crate::feat::workflow::protocol::command::InitWorkflow { name, workflow_id },
    )])
}

/// Populates the judge picker from scanned judge definitions.
fn load_judge_picker_entries(state: &mut AppState) {
    use crate::feat::judge::JudgePickerEntry;

    let active_id = state.session.active_session_id().clone();
    let theme = state.frontend.theme.clone();
    let entries: Vec<JudgePickerEntry> = state
        .context
        .judges
        .iter()
        .map(|j| {
            let already_attached = state.session.iter().any(|(_, s)| {
                s.judge()
                    .as_ref()
                    .is_some_and(|m| m.judge_name == j.name && m.origin_session == active_id)
            });
            JudgePickerEntry::from_judge(j, already_attached, theme.clone())
        })
        .collect();
    state.frontend.judge_picker.set_items(entries);
}

/// Confirms the selected judge and creates a judge session.
fn confirm_judge(state: &mut AppState) -> IntentResult {
    use crate::feat::session::chat_session::ChatSessionState;

    let Some(entry) = state.frontend.judge_picker.selected_item().cloned() else {
        return IntentResult::empty();
    };

    let active_id = state.session.active_session_id().clone();

    let Some(judge_def) = state
        .context
        .judges
        .iter()
        .find(|j| j.name == entry.name)
        .cloned()
    else {
        return IntentResult::empty();
    };

    // Check for existing detached judge with same name on this origin.
    let existing_id = state
        .session
        .iter()
        .find(|(_, s)| {
            s.judge().as_ref().is_some_and(|m| {
                m.origin_session == active_id && m.judge_name == entry.name && !m.is_attached
            })
        })
        .map(|(id, _)| id.clone());
    if let Some(existing_id) = existing_id {
        // Re-attach: set is_attached = true, activate the origin.
        if let Some(judge_session) = state.session.get_mut(&existing_id) {
            judge_session.set_judge_attached(true);
        }
        state.session.set_active(active_id);
        state.frontend.scope_stack.push(FocusScope::Input);
        state.frontend.judge_picker.reset();

        tracing::info!(
            judge_session = %existing_id,
            judge_name = %entry.name,
            "re-attached existing detached judge session"
        );
        return IntentResult::empty();
    }

    // Create a new judge session.
    let mut judge_session = ChatSessionState::new();
    let judge_id = judge_session.session_id().clone();

    // Set judge metadata — link to origin, mark as attached.
    judge_session.set_judge(crate::feat::judge::JudgeMeta {
        origin_session: active_id.clone(),
        is_attached: true,
        judge_name: entry.name.clone(),
    });

    // Set parent so it nests under the origin in the sidebar tree.
    judge_session.set_parent_session(active_id.clone());

    // Title the judge session after its definition name.
    judge_session.set_title(format!("judge/{}", &entry.name));

    // Set model: judge definition override takes priority, otherwise inherit from origin.
    if let Some(ref model) = judge_def.model {
        judge_session.set_model(model.clone());
    } else {
        let origin_model = state
            .session
            .get(&active_id)
            .expect("origin session should exist")
            .model()
            .to_owned();
        judge_session.set_model(origin_model);
    }

    // Pin the judge's body as a system entry at TOP position
    // so it survives compaction.
    let system_entry = crate::protocol::ChatEntry::system(&judge_def.body)
        .with_pin(crate::protocol::PinPosition::Top);
    judge_session.push_entry(system_entry);

    // Insert into session map and activate.
    state.session.insert(judge_session);
    state.session.set_active(judge_id.clone());
    state.frontend.scope_stack.pop();
    state.frontend.scope_stack.push(FocusScope::Input);

    IntentResult::with_commands(vec![Command::PersistSession(
        crate::feat::session_lifecycle::protocol::command::PersistSession {
            session_id: judge_id,
        },
    )])
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use super::*;
    use crate::feat::judge::Judge;
    use crate::feat::session::chat_session::ChatSessionState;
    use crate::protocol::PinPosition;
    use std::path::PathBuf;

    fn make_judge(name: &str, body: &str, model: Option<&str>) -> Judge {
        Judge {
            name: name.to_owned(),
            description: format!("{name} description"),
            body: body.to_owned(),
            model: model.map(std::borrow::ToOwned::to_owned),
            file_path: PathBuf::new(),
        }
    }

    fn setup_state_with_judge() -> AppState {
        let mut state = AppState::default();

        // Create and insert an active origin session.
        let origin = ChatSessionState::new();
        let origin_id = origin.session_id().clone();
        state.session.insert(origin);
        state.session.set_active(origin_id);

        // Add a judge definition.
        state
            .context
            .judges
            .push(make_judge("accuracy", "Check accuracy.", None));

        // Populate the picker and select the judge.
        load_judge_picker_entries(&mut state);
        // Select the first (and only) entry.
        state.frontend.judge_picker.move_down(1);

        state
    }

    #[rstest::rstest]
    fn confirm_judge_creates_session_with_correct_judge_meta() {
        let mut state = setup_state_with_judge();

        let origin_id = state.session.active_session_id().clone();
        let result = confirm_judge(&mut state);

        // Should have produced a PersistSession command.
        assert_eq!(result.commands.len(), 1);

        // The new active session should be a judge session.
        let active = state.active_session();
        let meta = active.judge().as_ref().expect("should have judge meta");
        assert_eq!(meta.judge_name, "accuracy");
        assert_eq!(meta.origin_session, origin_id);
        assert!(meta.is_attached);
    }

    #[rstest::rstest]
    fn confirm_judge_sets_parent_session_to_origin() {
        let mut state = setup_state_with_judge();
        let origin_id = state.session.active_session_id().clone();

        let _ = confirm_judge(&mut state);

        let active = state.active_session();
        assert_eq!(active.parent_session().as_ref(), Some(&origin_id));
    }

    #[rstest::rstest]
    fn confirm_judge_pins_system_entry_at_top() {
        let mut state = setup_state_with_judge();

        let _ = confirm_judge(&mut state);

        let active = state.active_session();
        let pinned: Vec<_> = active.pinned_entries();
        assert_eq!(pinned.len(), 1, "should have exactly one pinned entry");
        assert_eq!(pinned[0].pin_position, Some(PinPosition::Top));
        assert!(pinned[0].text().contains("Check accuracy."));
    }

    #[rstest::rstest]
    fn confirm_judge_re_attaches_detached_session() {
        // Given a detached judge session.
        let mut state = setup_state_with_judge();
        let origin_id = state.session.active_session_id().clone();
        confirm_judge(&mut state);

        // Find the judge session and detach it.
        let (judge_id, _) = state
            .session
            .iter()
            .find(|(_, s)| s.is_judge())
            .expect("judge session exists");
        let judge_id = judge_id.clone();
        state
            .session
            .get_mut(&judge_id)
            .expect("judge session")
            .set_judge_attached(false);

        // Switch back to origin session and re-populate picker.
        state.session.set_active(origin_id);
        load_judge_picker_entries(&mut state);
        state.frontend.judge_picker.move_down(1);

        // When confirming the picker again.
        let commands = confirm_judge(&mut state);

        // Then no new session is created.
        assert!(
            commands.commands.is_empty(),
            "should not create new session"
        );

        // And the judge session is re-attached.
        let judge_session = state
            .session
            .get(&judge_id)
            .expect("judge session should exist");
        assert!(
            judge_session
                .judge()
                .as_ref()
                .is_some_and(|m| m.is_attached)
        );
    }

    #[rstest::rstest]
    fn confirm_judge_sets_title_to_judge_name() {
        // Given state with a judge definition.
        let mut state = setup_state_with_judge();

        // When confirming the picker.
        let _ = confirm_judge(&mut state);

        // Then the judge session title includes the judge name.
        let active = state.active_session();
        assert_eq!(
            active.title().as_deref(),
            Some("judge/accuracy"),
            "title should be 'judge/<name>'"
        );
    }

    fn setup_state_with_judge_and_origin_model(
        origin_model: &str,
        judge_model: Option<&str>,
    ) -> AppState {
        let mut state = AppState::default();

        // Create origin session with specific model.
        let mut origin = ChatSessionState::new();
        let origin_id = origin.session_id().clone();
        origin.set_model(origin_model.to_owned());
        state.session.insert(origin);
        state.session.set_active(origin_id);

        // Add judge definition.
        state
            .context
            .judges
            .push(make_judge("accuracy", "Check accuracy.", judge_model));

        // Populate picker and select.
        load_judge_picker_entries(&mut state);
        state.frontend.judge_picker.move_down(1);

        state
    }

    #[rstest::rstest]
    fn confirm_judge_without_model_inherits_origin_model() {
        // Given an origin session with a specific model and a judge without model override.
        let mut state =
            setup_state_with_judge_and_origin_model("anthropic/claude-sonnet", None);

        // When confirming the judge picker.
        let _ = confirm_judge(&mut state);

        // Then the judge session inherits the origin's model.
        let active = state.active_session();
        assert_eq!(active.model(), "anthropic/claude-sonnet");
    }

    #[rstest::rstest]
    fn confirm_judge_with_model_override_ignores_origin() {
        // Given an origin session with a model and a judge with its own model override.
        let mut state = setup_state_with_judge_and_origin_model(
            "anthropic/claude-sonnet",
            Some("anthropic/claude-haiku"),
        );

        // When confirming the judge picker.
        let _ = confirm_judge(&mut state);

        // Then the judge session uses its own model, not the origin's.
        let active = state.active_session();
        assert_eq!(active.model(), "anthropic/claude-haiku");
    }
}
