//! Session management — session lifecycle, persistence, and loading.
//!
//! Provides persistence types ([`ChatSessionState`], [`SessionStore`], etc.)
//! used by the session actor, services container, and component crate.
//! Also contains the session actor, intent handlers, validators, entry loaders,
//! and picker rendering.

pub mod chat_history;
pub mod session_store;
pub mod session_summary;

pub mod chat_entry;

#[cfg(test)]
mod chat_entry_tests;
pub mod chat_session;
pub mod entries;
pub mod fork_entry;
pub mod intent;
pub mod picker_entry;
pub mod profile;
pub mod protocol;
pub mod render;
pub mod session_actor;
pub mod token_stats;
#[cfg(test)]
mod token_stats_tests;
pub(crate) mod tool_result_status;
pub mod validator;

pub use chat_session::{ChatSessionState, SessionCore, SessionUi};
pub use profile::SessionProfile;
pub use session_store::{
    PoolConfig, SessionStore, SessionStoreError, SessionStoreService, SqliteSessionStore,
};
pub use session_summary::SessionSummary;
pub use token_stats::{AggregatedTokenStats, TokenRecord, TokenStats, aggregate_session_stats};

/// Returns a welcome message entry for display at application startup.
///
/// Lists common keyboard shortcuts so new users know how to interact
/// with the application. Uses [`crate::protocol::ChatEntry::info`] so the message is
/// excluded from LLM context.
pub fn welcome_msg() -> crate::protocol::ChatEntry {
    crate::protocol::ChatEntry::transient(
        "\
**Welcome to nullslop!**

\
**   i       ** — enter insert mode
\
**   ESC     ** — cancel stream (press twice)
\
**   ?       ** — show all shortcuts
\
**   q       ** — quit

\
**   Spatial Navigation**
\
**   Ctrl+K  ** — focus left (input or chat history)
\
**   Ctrl+L  ** — focus right (sidebar)
\
**   Ctrl+J  ** — focus input box",
    )
}

/// Returns a guidance message for when no API keys are found.
///
/// Instructs the user to create a `.env` file and shows the path to
/// `providers.toml` for reference. Uses [`crate::protocol::ChatEntry::info`]
/// so the message is excluded from LLM context.
pub fn no_api_keys_msg() -> crate::protocol::ChatEntry {
    let config_path = crate::feat::provider_infra::config_path()
        .to_string_lossy()
        .into_owned();

    let content = format!(
        "\
**No API keys found**

\
Create a `.env` file in your working directory with your API keys.
\
See `{config_path}` for available environment variables."
    );

    crate::protocol::ChatEntry::transient(content)
}

#[cfg(test)]
mod welcome_tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use super::*;
    use crate::protocol::ChatEntryKind;

    #[rstest::rstest]
    fn welcome_msg_is_transient_entry() {
        // When creating the welcome message.
        let entry = welcome_msg();

        // Then it is a Transient entry.
        assert!(matches!(entry.kind, ChatEntryKind::Transient(_)));
    }

    #[rstest::rstest]
    fn welcome_msg_contains_all_shortcuts() {
        // When creating the welcome message.
        let entry = welcome_msg();

        // Then it mentions all key shortcuts.
        let text = entry.text();
        assert!(text.contains('i'), "should mention i");
        assert!(text.contains("ESC"), "should mention ESC");
        assert!(text.contains('?'), "should mention ?");
        assert!(text.contains('q'), "should mention q");
        assert!(text.contains("Ctrl+K"), "should mention Ctrl+K");
        assert!(text.contains("Ctrl+L"), "should mention Ctrl+L");
        assert!(text.contains("Ctrl+J"), "should mention Ctrl+J");
        assert!(
            text.contains("Spatial Navigation"),
            "should mention Spatial Navigation"
        );
    }

    #[rstest::rstest]
    fn no_api_keys_msg_is_transient_entry() {
        // When creating the no-api-keys message.
        let entry = no_api_keys_msg();

        // Then it is a Transient entry.
        assert!(matches!(entry.kind, ChatEntryKind::Transient(_)));
    }

    #[rstest::rstest]
    fn no_api_keys_msg_contains_guidance() {
        // When creating the no-api-keys message.
        let entry = no_api_keys_msg();

        // Then it mentions guidance keywords.
        let text = entry.text();
        assert!(text.contains("No API keys found"), "should mention header");
        assert!(text.contains(".env"), "should mention .env");
        assert!(
            text.contains("providers.toml"),
            "should mention providers.toml"
        );
    }
}
