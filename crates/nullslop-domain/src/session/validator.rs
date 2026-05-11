//! Session and model intent validators.
//!
//! Validators for model refresh, prompt template rescan, and session creation.

use crate::component::AppState;
use wherror::Error;

/// Errors from validating a RefreshModels intent.
#[derive(Debug, Error)]
#[error(debug)]
pub enum RefreshModelsError {
    /// No provider is configured.
    NoProvider,
}

/// Validates the RefreshModels intent.
///
/// Returns an error if no provider is configured.
///
/// # Errors
///
/// Returns an error if no provider is configured.
pub fn validate_refresh_models(state: &AppState) -> Result<(), RefreshModelsError> {
    if state.provider.active_provider == crate::component::NO_PROVIDER_ID {
        return Err(RefreshModelsError::NoProvider);
    }
    Ok(())
}

/// Errors from validating a RescanPromptTemplates intent.
#[derive(Debug, Error)]
#[error(debug)]
pub enum RescanPromptTemplatesError {
    /// Prompt templates directory is not configured.
    NotConfigured,
}

/// Validates the RescanPromptTemplates intent.
///
/// Always succeeds for now. The error variant exists for future use.
///
/// # Errors
///
/// Returns an error if prompt templates directory is not configured.
pub fn validate_rescan_prompt_templates(
    _state: &AppState,
) -> Result<(), RescanPromptTemplatesError> {
    Ok(())
}

/// Errors from validating a SessionNew intent.
#[derive(Debug, Error)]
#[error(debug)]
pub enum SessionNewError {
    /// A picker is currently active.
    PickerActive,
}

/// Validates the SessionNew intent.
///
/// Returns an error if a picker is currently active.
///
/// # Errors
///
/// Returns an error if a picker is currently active.
pub fn validate_session_new(state: &AppState) -> Result<(), SessionNewError> {
    if state.frontend.active_picker_kind.is_some() {
        return Err(SessionNewError::PickerActive);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::protocol::PickerKind;

    use super::*;

    // --- RefreshModels tests ---

    #[rstest::rstest]
    fn refresh_models_succeeds_with_provider() {
        // Given a state with a configured provider.
        let mut state = AppState::default();
        state.provider.active_provider = "ollama".to_owned();

        // When validating refresh models.
        let result = validate_refresh_models(&state);

        // Then it succeeds.
        assert!(result.is_ok());
    }

    #[rstest::rstest]
    fn refresh_models_fails_with_no_provider() {
        // Given a state with the default no-provider ID.
        let state = AppState::default();

        // When validating refresh models.
        let result = validate_refresh_models(&state);

        // Then it returns NoProvider error.
        assert!(matches!(result, Err(RefreshModelsError::NoProvider)));
    }

    // --- SessionNew tests ---

    #[rstest::rstest]
    fn session_new_succeeds_when_no_picker_active() {
        // Given a state with no active picker.
        let state = AppState::default();

        // When validating session new.
        let result = validate_session_new(&state);

        // Then it succeeds.
        assert!(result.is_ok());
    }

    #[rstest::rstest]
    fn session_new_fails_when_picker_active() {
        // Given a state with an active picker.
        let mut state = AppState::default();
        state.frontend.active_picker_kind = Some(PickerKind::Provider);

        // When validating session new.
        let result = validate_session_new(&state);

        // Then it returns PickerActive error.
        assert!(matches!(result, Err(SessionNewError::PickerActive)));
    }
}
