//! Per-session storage state shared by the kernel and session-store contract.

use serde::{Deserialize, Serialize};

/// Whether a session is loaded in memory or archived at rest.
///
/// `Loaded` sessions are available for interaction. `Archived` sessions remain in
/// persistent storage but are hidden from the active session list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    /// The session is loaded and available for interaction.
    #[default]
    Loaded,
    /// The session is archived in persistent storage.
    Archived,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "test code")]

    use super::SessionState;

    #[rstest::rstest]
    fn session_state_defaults_to_loaded() {
        // Given no explicit state.

        // When creating the default state.
        let state = SessionState::default();

        // Then the session is loaded.
        assert_eq!(state, SessionState::Loaded);
    }

    #[rstest::rstest]
    #[case(SessionState::Loaded, "\"loaded\"")]
    #[case(SessionState::Archived, "\"archived\"")]
    fn session_state_serializes_as_snake_case(#[case] state: SessionState, #[case] expected: &str) {
        // Given a session state.

        // When serializing it.
        let json = serde_json::to_string(&state).unwrap();

        // Then the JSON uses the snake_case variant name.
        assert_eq!(json, expected);
    }

    #[rstest::rstest]
    #[case("\"loaded\"", SessionState::Loaded)]
    #[case("\"archived\"", SessionState::Archived)]
    fn session_state_deserializes_from_snake_case(
        #[case] raw: &str,
        #[case] expected: SessionState,
    ) {
        // Given a persisted snake_case state name.

        // When deserializing it.
        let state: SessionState = serde_json::from_str(raw).unwrap();

        // Then the expected state is restored.
        assert_eq!(state, expected);
    }
}
