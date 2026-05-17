//! Session management — session lifecycle, persistence, and loading.
//!
//! Provides persistence types ([`ChatSessionState`], [`SessionStore`], etc.)
//! used by the session actor, services container, and component crate.
//! Also contains the session actor, intent handlers, validators, entry loaders,
//! and picker rendering.

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
    crate::protocol::ChatEntry::info(
        "Welcome to nullslop!\n\
         \n\
         \u{2003}i       \u{2014} enter insert mode\n\
         \u{2003}Ctrl+J  \u{2014} enter insert mode\n\
         \u{2003}?       \u{2014} show all shortcuts\n\
         \u{2003}q       \u{2014} quit\n\
         \u{2003}Ctrl+K  \u{2014} back to normal mode\n\
         \u{2003}Ctrl+L  \u{2014} toggle sidebar\n\
         \u{2003}ESC     \u{2014} cancel stream",
    )
}

#[cfg(test)]
mod welcome_tests {
    use super::*;
    use crate::protocol::ChatEntryKind;

    #[rstest::rstest]
    fn welcome_msg_is_info_entry() {
        // When creating the welcome message.
        let entry = welcome_msg();

        // Then it is an Info entry.
        assert!(matches!(entry.kind, ChatEntryKind::Info(_)));
    }

    #[rstest::rstest]
    fn welcome_msg_contains_all_shortcuts() {
        // When creating the welcome message.
        let entry = welcome_msg();

        // Then it mentions all key shortcuts.
        let text = entry.text();
        assert!(text.contains('i'), "should mention i");
        assert!(text.contains("Ctrl+J"), "should mention Ctrl+J");
        assert!(text.contains('?'), "should mention ?");
        assert!(text.contains('q'), "should mention q");
        assert!(text.contains("Ctrl+K"), "should mention Ctrl+K");
        assert!(text.contains("Ctrl+L"), "should mention Ctrl+L");
        assert!(text.contains("ESC"), "should mention ESC");
    }
}
