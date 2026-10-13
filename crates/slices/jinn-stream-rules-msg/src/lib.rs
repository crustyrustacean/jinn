//! Stream-rules crossing contracts — the message that re-arms the live
//! matcher.
//!
//! The compiled stream-rule matcher lives in the `jinn-stream-rules` slice's
//! cell, installed once at activation. This crate carries the one command
//! that asks for a re-install mid-session, so a configuration write can
//! reach the running matcher without a restart: the stream-rules slice
//! handles the command, and any writer of the configuration publishes it.
//! The crate is the pure contract — data only, no matcher, no actor.

/// Rebuild and re-install the live stream-rule matcher from the current
/// configuration.
///
/// Published by a configuration writer (today: the stream-rule picker's
/// confirm) after the write has landed, so the matcher a fresh stream sees
/// is compiled from what the file now holds. Handled by the
/// `jinn-stream-rules` slice's matcher actor, which re-runs the same read
/// + compile + install path activation ran.
///
/// Carries no payload: there is exactly one matcher cell and exactly one
/// source of rules, so the command names nothing that could be otherwise.
/// Rebuilding after every confirm is the whole cost — an idempotent
/// re-compile, not a diff.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, trouper::schema::Command)]
#[schema(description = "Rebuild the live stream-rule matcher from the current configuration.")]
pub struct RefreshStreamRuleMatcher;

impl jinn_slices::BusMessage for RefreshStreamRuleMatcher {}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        reason = "test module, panics are acceptable"
    )]
    use super::*;
    use trouper::schema::Schema;

    #[rstest::rstest]
    fn the_command_is_its_own_schema_named_after_the_type() {
        // Given the refresh command.
        // When asking for its schema id.
        // Then it is named after the type — the fabric routes by this id,
        // so a rename would silently orphan a published command's
        // subscribers.
        assert_eq!(
            RefreshStreamRuleMatcher::schema_id().name(),
            "RefreshStreamRuleMatcher"
        );
    }

    #[rstest::rstest]
    fn the_command_roundtrips_the_wire() {
        // Given the refresh command.
        let command = RefreshStreamRuleMatcher;

        // When it crosses the wire as JSON and back.
        let payload = serde_json::to_value(&command).expect("serialize");
        let decoded: RefreshStreamRuleMatcher =
            serde_json::from_value(payload).expect("deserialize");

        // Then the decoded command is the same unit — nothing to lose in
        // transit, which is why it carries no payload.
        assert_eq!(
            serde_json::to_value(&decoded).expect("serialize"),
            serde_json::to_value(&command).expect("serialize")
        );
    }
}
