//! Session identity vocabulary shared across session contracts.

use serde::{Deserialize, Serialize};

/// How a session came into being.
///
/// A session's place in the tree is represented separately by its parent ID and
/// fork ordinal; this enum records the kind of creation path.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionOrigin {
    /// Created by the user, including restart restoration of an existing session.
    #[default]
    User,
    /// Created by forking an existing session.
    Fork,
    /// Spawned as a child session by another session.
    Subagent,
    /// A peer session that references a parent and re-runs when the parent's
    /// turn completes. Inherits the parent's environment but never its
    /// conversation, and reports what it concludes back to the user.
    Attendant,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "test code")]

    use super::SessionOrigin;

    #[rstest::rstest]
    fn session_origin_defaults_to_user() {
        // Given no explicit origin.

        // When creating the default origin.
        let origin = SessionOrigin::default();

        // Then the session was created by the user.
        assert_eq!(origin, SessionOrigin::User);
    }

    #[rstest::rstest]
    #[case(SessionOrigin::User, "\"user\"")]
    #[case(SessionOrigin::Fork, "\"fork\"")]
    #[case(SessionOrigin::Subagent, "\"subagent\"")]
    #[case(SessionOrigin::Attendant, "\"attendant\"")]
    fn session_origin_serializes_as_snake_case(
        #[case] origin: SessionOrigin,
        #[case] expected: &str,
    ) {
        // Given a session origin.

        // When serializing it.
        let json = serde_json::to_string(&origin).unwrap();

        // Then the JSON uses the snake_case variant name.
        assert_eq!(json, expected);
    }

    #[rstest::rstest]
    #[case("\"user\"", SessionOrigin::User)]
    #[case("\"fork\"", SessionOrigin::Fork)]
    #[case("\"subagent\"", SessionOrigin::Subagent)]
    #[case("\"attendant\"", SessionOrigin::Attendant)]
    fn session_origin_deserializes_from_snake_case(
        #[case] raw: &str,
        #[case] expected: SessionOrigin,
    ) {
        // Given a persisted snake_case origin name.

        // When deserializing it.
        let origin: SessionOrigin = serde_json::from_str(raw).unwrap();

        // Then the expected origin is restored.
        assert_eq!(origin, expected);
    }
}
