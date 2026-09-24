//! Lifecycle script progression shared by session lifecycle contracts and the kernel.

use serde::{Deserialize, Serialize};

/// The lifecycle script progression for a session.
///
/// One-way transitions are enforced by [`advance_after_setup`](Self::advance_after_setup)
/// and [`advance_after_teardown`](Self::advance_after_teardown). These methods are called
/// only after the corresponding script succeeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleScriptState {
    /// No lifecycle script has run.
    #[default]
    NothingRan,
    /// Session setup completed successfully.
    SetupRan,
    /// Session teardown completed successfully.
    TeardownRan,
}

impl LifecycleScriptState {
    /// Transition `NothingRan` to `SetupRan`.
    ///
    /// Soft guard: if current state is not `NothingRan`, logs a warning and returns.
    pub fn advance_after_setup(&mut self) {
        if !matches!(self, Self::NothingRan) {
            tracing::warn!(current = ?self, "advance_after_setup: expected NothingRan, ignoring");
            return;
        }
        *self = Self::SetupRan;
    }

    /// Transition `SetupRan` to `TeardownRan`.
    ///
    /// Soft guard: if current state is not `SetupRan`, logs a warning and returns.
    pub fn advance_after_teardown(&mut self) {
        if !matches!(self, Self::SetupRan) {
            tracing::warn!(current = ?self, "advance_after_teardown: expected SetupRan, ignoring");
            return;
        }
        *self = Self::TeardownRan;
    }
}

impl std::fmt::Display for LifecycleScriptState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let rendered = match self {
            Self::NothingRan => "nothing_ran",
            Self::SetupRan => "setup_ran",
            Self::TeardownRan => "teardown_ran",
        };
        f.write_str(rendered)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "test code")]

    use super::LifecycleScriptState;

    #[rstest::rstest]
    fn advance_after_setup_transitions_nothing_to_setup() {
        // Given NothingRan.
        let mut state = LifecycleScriptState::NothingRan;

        // When advancing after setup.
        state.advance_after_setup();

        // Then state is SetupRan.
        assert_eq!(state, LifecycleScriptState::SetupRan);
    }

    #[rstest::rstest]
    fn advance_after_teardown_transitions_setup_to_teardown() {
        // Given SetupRan.
        let mut state = LifecycleScriptState::SetupRan;

        // When advancing after teardown.
        state.advance_after_teardown();

        // Then state is TeardownRan.
        assert_eq!(state, LifecycleScriptState::TeardownRan);
    }

    #[rstest::rstest]
    fn advance_after_setup_is_noop_from_setup_ran() {
        // Given SetupRan.
        let mut state = LifecycleScriptState::SetupRan;

        // When advancing after setup again.
        state.advance_after_setup();

        // Then state stays SetupRan.
        assert_eq!(state, LifecycleScriptState::SetupRan);
    }

    #[rstest::rstest]
    fn advance_after_setup_is_noop_from_teardown_ran() {
        // Given TeardownRan.
        let mut state = LifecycleScriptState::TeardownRan;

        // When advancing after setup.
        state.advance_after_setup();

        // Then state stays TeardownRan.
        assert_eq!(state, LifecycleScriptState::TeardownRan);
    }

    #[rstest::rstest]
    fn advance_after_teardown_is_noop_from_nothing_ran() {
        // Given NothingRan.
        let mut state = LifecycleScriptState::NothingRan;

        // When advancing after teardown.
        state.advance_after_teardown();

        // Then state stays NothingRan.
        assert_eq!(state, LifecycleScriptState::NothingRan);
    }

    #[rstest::rstest]
    fn advance_after_teardown_is_noop_from_teardown_ran() {
        // Given TeardownRan.
        let mut state = LifecycleScriptState::TeardownRan;

        // When advancing after teardown again.
        state.advance_after_teardown();

        // Then state stays TeardownRan.
        assert_eq!(state, LifecycleScriptState::TeardownRan);
    }

    #[rstest::rstest]
    #[case(LifecycleScriptState::NothingRan, "nothing_ran")]
    #[case(LifecycleScriptState::SetupRan, "setup_ran")]
    #[case(LifecycleScriptState::TeardownRan, "teardown_ran")]
    fn lifecycle_script_state_displays_serde_name(
        #[case] state: LifecycleScriptState,
        #[case] expected: &str,
    ) {
        // Given each lifecycle script state variant.

        // When formatting it.
        let rendered = state.to_string();

        // Then it renders as the snake_case serde name.
        assert_eq!(rendered, expected);
    }

    #[rstest::rstest]
    #[case(LifecycleScriptState::NothingRan, "\"nothing_ran\"")]
    #[case(LifecycleScriptState::SetupRan, "\"setup_ran\"")]
    #[case(LifecycleScriptState::TeardownRan, "\"teardown_ran\"")]
    fn lifecycle_script_state_serializes_as_snake_case(
        #[case] state: LifecycleScriptState,
        #[case] expected: &str,
    ) {
        // Given each lifecycle script state variant.

        // When serializing it.
        let json = serde_json::to_string(&state).unwrap();

        // Then the JSON uses the snake_case variant name.
        assert_eq!(json, expected);
    }
}
