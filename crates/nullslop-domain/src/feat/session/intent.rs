//! Session management intent handlers — session creation, model refresh, and prompt template rescan.

use crate::common::app_state::AppState;
use crate::protocol::{ChatEntry, Command, IntentResult, SessionId};

use super::validator;

/// Creates a new chat session, replacing the active one.
pub fn handle_session_new(state: &mut AppState) -> IntentResult {
    if validator::validate_session_new(state).is_err() {
        return IntentResult::empty();
    }

    state.session.sessions.remove(&state.session.active_session);

    let new_id = SessionId::new();
    state.session.sessions.insert(
        new_id.clone(),
        crate::feat::session::chat_session::ChatSessionState::new(),
    );
    state.session.active_session = new_id;
    state.frontend.scope_stack.clear_overlays();

    IntentResult::empty()
}

/// Refreshes the model list from the active provider.
pub fn handle_refresh_models(state: &mut AppState) -> IntentResult {
    if validator::validate_refresh_models(state).is_err() {
        return IntentResult::empty();
    }

    state
        .active_session_mut()
        .push_entry(ChatEntry::system("Refreshing models..."));

    IntentResult::with_commands(vec![Command::RefreshModels])
}

/// Rescans prompt templates from disk.
pub fn handle_rescan_prompt_templates(state: &mut AppState) -> IntentResult {
    let _ = validator::validate_rescan_prompt_templates(state);

    state
        .active_session_mut()
        .push_entry(ChatEntry::system("Rescanning prompt templates..."));

    IntentResult::with_commands(vec![Command::RescanPromptTemplates])
}

#[cfg(test)]
mod tests {
    use crate::common::app_state::{AppState, FrontendState};
    use crate::feat::provider::ProviderState;
    use crate::protocol::{ChatEntry, Command, Mode, PickerKind};

    use super::*;

    #[rstest::rstest]
    fn session_new_creates_fresh_session() {
        // Given a state with an existing session.
        let mut state = AppState::default();
        let old_id = state.session.active_session.clone();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("old"));

        // When handling SessionNew.
        let _result = handle_session_new(&mut state);

        // Then a new session is created.
        assert_ne!(state.session.active_session, old_id);
        assert!(state.active_session().history().is_empty());
        assert!(!state.frontend.scope_stack.is_picker());
    }

    #[rstest::rstest]
    fn session_new_noop_when_picker_active() {
        // Given a state with an active picker.
        let mut state = AppState::default();
        state.frontend.scope_stack.push(
            crate::common::app_state::FocusScope::Picker { kind: PickerKind::Provider },
        );
        let old_id = state.session.active_session.clone();

        // When handling SessionNew.
        let result = handle_session_new(&mut state);

        // Then nothing changed.
        assert_eq!(state.session.active_session, old_id);
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn refresh_models_posts_system_message_and_returns_command() {
        // Given a state with a provider.
        let mut state = AppState {
            provider: ProviderState {
                active_provider: "ollama".to_owned(),
                ..ProviderState::default()
            },
            ..Default::default()
        };
        let initial_len = state.active_session().history().len();

        // When handling RefreshModels.
        let result = handle_refresh_models(&mut state);

        // Then a system message was posted.
        assert_eq!(state.active_session().history().len(), initial_len + 1);
        // And a RefreshModels command is returned.
        assert!(
            result
                .commands
                .iter()
                .any(|c| matches!(c, Command::RefreshModels))
        );
    }

    #[rstest::rstest]
    fn refresh_models_noop_with_no_provider() {
        // Given a state with no provider.
        let mut state = AppState::default();

        // When handling RefreshModels.
        let result = handle_refresh_models(&mut state);

        // Then no commands.
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn rescan_prompt_templates_posts_system_message_and_returns_command() {
        // Given a default state.
        let mut state = AppState::default();
        let initial_len = state.active_session().history().len();

        // When handling RescanPromptTemplates.
        let result = handle_rescan_prompt_templates(&mut state);

        // Then a system message was posted.
        assert_eq!(state.active_session().history().len(), initial_len + 1);
        // And a RescanPromptTemplates command is returned.
        assert!(
            result
                .commands
                .iter()
                .any(|c| matches!(c, Command::RescanPromptTemplates))
        );
    }
}
