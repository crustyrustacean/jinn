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
use crate::feat::context::protocol::command::{
    LoadContextStrategyPickerEntries, SwitchPromptStrategy,
};
use crate::feat::preferences_actor::protocol::command::{PreferenceUpdate, UpdatePreferences};
use crate::feat::provider::protocol::command::{LoadProviderPickerEntries, ProviderSwitch};
use crate::feat::session::fork_entry::ForkEntry;
use crate::feat::session::protocol::load_session_picker_entries::LoadSessionPickerEntries;
use crate::feat::session::protocol::session_fork_requested::SessionForkRequested;
use crate::feat::session::protocol::session_load_requested::SessionLoadRequested;
use crate::protocol::{ChatEntryKind, Command, Intent, IntentResult, PickerKind};

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
        PickerKind::ContextAssembly => {
            state.frontend.context_strategy_picker.reset();
        }
        PickerKind::Keymap => {
            state.frontend.keymap_picker.reset();
            state.frontend.keymap_picker_show_all = false;
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
        PickerKind::SessionFork => {
            state.frontend.fork_picker.reset();
            state.frontend.fork_show_user = true;
            state.frontend.fork_show_assistant = true;
        }
        PickerKind::SessionLifecycle => {
            state.frontend.session_lifecycle_picker.reset();
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
        PickerKind::ContextAssembly => {
            IntentResult::with_commands(vec![Command::LoadContextStrategyPickerEntries(
                LoadContextStrategyPickerEntries,
            )])
        }
        PickerKind::Persona => {
            IntentResult::with_commands(vec![Command::LoadPersonaPickerEntries(
                LoadPersonaPickerEntries,
            )])
        }
        PickerKind::Keymap | PickerKind::Theme => IntentResult::empty(),
        PickerKind::SessionFork => {
            // Populate from active session history (synchronous, no actor needed).
            let entries = build_fork_entries(state);
            state.frontend.all_fork_entries.clone_from(&entries);
            state.frontend.fork_picker.set_items(entries);
            IntentResult::empty()
        }
        PickerKind::SessionLifecycle => {
            // Populate from user preferences + implicit blank lifecycle.
            load_lifecycle_picker_entries(state);
            IntentResult::empty()
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
/// Returns `(IntentResult, Option<Intent>)`. For Provider, ContextAssembly, and
/// Session pickers, the second element is `None`. For Keymap picker, returns
/// `(IntentResult::empty(), Some(selected_intent))` so the caller can re-dispatch.
pub fn handle_picker_confirm(state: &mut AppState) -> (IntentResult, Option<Intent>) {
    if validator::validate_picker_confirm(state).is_err() {
        return (IntentResult::empty(), None);
    }

    match state.frontend.scope_stack.picker_kind().copied() {
        Some(PickerKind::Provider) => (confirm_provider(state), None),
        Some(PickerKind::ContextAssembly) => (confirm_strategy(state), None),
        Some(PickerKind::Keymap) => confirm_keymap(state),
        Some(PickerKind::Session) => (confirm_session(state), None),
        Some(PickerKind::Persona) => (confirm_persona(state), None),
        Some(PickerKind::Theme) => (confirm_theme(state), None),
        Some(PickerKind::SessionFork) => (confirm_session_fork(state), None),
        Some(PickerKind::SessionLifecycle) => (confirm_session_lifecycle(state), None),
        None => (IntentResult::empty(), None),
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

/// Toggles the keymap scope filter between showing all entries and the
/// current scope's entries only.
pub fn handle_toggle_keymap_scope_filter(state: &mut AppState) -> IntentResult {
    validator::validate_toggle_keymap_scope_filter(state);

    state.frontend.keymap_picker_show_all = !state.frontend.keymap_picker_show_all;

    let scope = state
        .frontend
        .scope_stack
        .parent()
        .map(std::string::ToString::to_string)
        .unwrap_or_default();

    let filtered: Vec<_> = if state.frontend.keymap_picker_show_all {
        state.frontend.all_keymap_entries.clone()
    } else {
        state
            .frontend
            .all_keymap_entries
            .iter()
            .filter(|e| e.scope == scope)
            .cloned()
            .collect()
    };

    state.frontend.keymap_picker.set_items(filtered);
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
    let session_id = state.session.active_session.clone();

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

/// Confirms the selected strategy and dispatches a switch command.
fn confirm_strategy(state: &mut AppState) -> IntentResult {
    let Some(entry) = state.frontend.context_strategy_picker.selected_item() else {
        return IntentResult::empty();
    };
    let strategy_id = entry.strategy_id.clone();
    let session_id = state.session.active_session.clone();

    state.frontend.scope_stack.pop();
    IntentResult::with_commands(vec![
        Command::SwitchPromptStrategy(SwitchPromptStrategy {
            session_id,
            strategy_id: strategy_id.clone(),
        }),
        Command::UpdatePreferences(UpdatePreferences {
            updates: vec![PreferenceUpdate::SetLastStrategy(Some(
                strategy_id.as_str().to_owned(),
            ))],
        }),
    ])
}

/// Confirms a keymap selection. Returns the selected intent for the caller
/// to re-dispatch, rather than dispatching it directly (avoids circular dep).
fn confirm_keymap(state: &mut AppState) -> (IntentResult, Option<Intent>) {
    let Some(entry) = state.frontend.keymap_picker.selected_item() else {
        return (IntentResult::empty(), None);
    };
    let intent = entry.command.clone();

    state.frontend.scope_stack.pop();
    (IntentResult::empty(), Some(intent))
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

/// Builds fork entries from the active session's history.
///
/// Includes only User and Assistant entries, preserving their ordinal positions.
fn build_fork_entries(state: &AppState) -> Vec<ForkEntry> {
    let session = state.active_session();
    session
        .history()
        .iter()
        .enumerate()
        .filter(|(_, e)| {
            matches!(
                &e.kind,
                ChatEntryKind::User { .. } | ChatEntryKind::Assistant(_)
            )
        })
        .map(|(i, e)| ForkEntry {
            ordinal: i,
            text: e.text(),
            is_user: matches!(&e.kind, ChatEntryKind::User { .. }),
            theme: state.frontend.theme.clone(),
        })
        .collect()
}

/// Confirms the selected fork entry and dispatches a fork command.
fn confirm_session_fork(state: &mut AppState) -> IntentResult {
    let Some(entry) = state.frontend.fork_picker.selected_item() else {
        return IntentResult::empty();
    };
    let source_session_id = state.session.active_session.clone();
    let at_ordinal = entry.ordinal;

    state.session.begin_load(source_session_id.clone());
    state.frontend.scope_stack.pop();

    IntentResult::with_commands(vec![Command::SessionForkRequested(SessionForkRequested {
        source_session_id,
        at_ordinal,
    })])
}

/// Toggles user message visibility in the fork picker.
///
/// No-op if the fork picker is not active.
pub fn handle_toggle_fork_user_filter(state: &mut AppState) -> IntentResult {
    if state.frontend.scope_stack.picker_kind().copied() != Some(PickerKind::SessionFork) {
        return IntentResult::empty();
    }

    state.frontend.fork_show_user = !state.frontend.fork_show_user;
    apply_fork_filters(state);
    IntentResult::empty()
}

/// Toggles assistant message visibility in the fork picker.
///
/// No-op if the fork picker is not active.
pub fn handle_toggle_fork_assistant_filter(state: &mut AppState) -> IntentResult {
    if state.frontend.scope_stack.picker_kind().copied() != Some(PickerKind::SessionFork) {
        return IntentResult::empty();
    }

    state.frontend.fork_show_assistant = !state.frontend.fork_show_assistant;
    apply_fork_filters(state);
    IntentResult::empty()
}

/// Applies fork filter flags to rebuild the displayed entries.
fn apply_fork_filters(state: &mut AppState) {
    let filtered: Vec<ForkEntry> = state
        .frontend
        .all_fork_entries
        .iter()
        .filter(|e| {
            (e.is_user && state.frontend.fork_show_user)
                || (!e.is_user && state.frontend.fork_show_assistant)
        })
        .cloned()
        .collect();
    state.frontend.fork_picker.set_items(filtered);
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
            .setup_command
            .as_ref()
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
            .and_then(|l| l.setup_command.as_ref())
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
