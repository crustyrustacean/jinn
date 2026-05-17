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
#[cfg(test)]
mod token_stats_tests;
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
    use ratatui::style::{Modifier, Style};
    use ratatui::text::{Line, Span};

    let muted = Style::default().fg(crate::feat::theme::default_theme().muted_text);
    let primary = Style::default().fg(crate::feat::theme::default_theme().primary_text);
    let bold = primary.add_modifier(Modifier::BOLD);

    let dim = Style::default().fg(crate::feat::theme::default_theme().muted_text);

    crate::protocol::ChatEntry::info(vec![
        Line::from(Span::styled("Welcome to nullslop!", bold)),
        Line::from(""),
        Line::from(vec![
            Span::styled("   i       ", bold),
            Span::styled("— ", muted),
            Span::styled("enter insert mode", primary),
        ]),
        Line::from(vec![
            Span::styled("   ESC     ", bold),
            Span::styled("— ", muted),
            Span::styled("cancel stream (press twice)", primary),
        ]),
        Line::from(vec![
            Span::styled("   ?       ", bold),
            Span::styled("— ", muted),
            Span::styled("show all shortcuts", primary),
        ]),
        Line::from(vec![
            Span::styled("   q       ", bold),
            Span::styled("— ", muted),
            Span::styled("quit", primary),
        ]),
        Line::from(""),
        Line::from(Span::styled("   Spatial Navigation", dim)),
        Line::from(vec![
            Span::styled("   Ctrl+K  ", bold),
            Span::styled("— ", muted),
            Span::styled("focus left (input or chat history)", primary),
        ]),
        Line::from(vec![
            Span::styled("   Ctrl+L  ", bold),
            Span::styled("— ", muted),
            Span::styled("focus right (sidebar)", primary),
        ]),
        Line::from(vec![
            Span::styled("   Ctrl+J  ", bold),
            Span::styled("— ", muted),
            Span::styled("focus input box", primary),
        ]),
    ])
}

/// Returns a guidance message for when no API keys are found.
///
/// Instructs the user to create a `.env` file and shows the path to
/// `providers.toml` for reference. Uses [`crate::protocol::ChatEntry::info`]
/// so the message is excluded from LLM context.
pub fn no_api_keys_msg() -> crate::protocol::ChatEntry {
    use ratatui::style::{Modifier, Style};
    use ratatui::text::{Line, Span};

    let primary = Style::default().fg(crate::feat::theme::default_theme().primary_text);
    let bold = primary.add_modifier(Modifier::BOLD);

    let config_path = crate::feat::provider_infra::config_path()
        .to_string_lossy()
        .into_owned();

    crate::protocol::ChatEntry::info(vec![
        Line::from(Span::styled("No API keys found", bold)),
        Line::from(""),
        Line::from(vec![
            Span::styled("Create a ", primary),
            Span::styled(".env", bold),
            Span::styled(
                " file in your working directory with your API keys.",
                primary,
            ),
        ]),
        Line::from(vec![
            Span::styled("See ", primary),
            Span::styled(config_path, bold),
            Span::styled(" for available environment variables.", primary),
        ]),
    ])
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
    fn no_api_keys_msg_is_info_entry() {
        // When creating the no-api-keys message.
        let entry = no_api_keys_msg();

        // Then it is an Info entry.
        assert!(matches!(entry.kind, ChatEntryKind::Info(_)));
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
