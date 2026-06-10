#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::indexing_slicing,
    reason = "test code"
)]

//! Tests for `EventMsg` and `CommandMsg` derive macro code generation.

// Import from protocol:: which re-exports both the derive macros and the traits.
use crate::protocol::{CommandMsg, EventMsg};

// -- Test-only structs with derive macros applied --

/// Test fixture: a simple event struct with `EventMsg` derived.
#[derive(Debug, Clone, EventMsg)]
#[event_msg("test_mod")]
struct TestEvent;

/// Test fixture: a simple command struct with `CommandMsg` derived.
#[derive(Debug, Clone, CommandMsg)]
#[cmd("test_mod")]
struct TestCommand;

#[rstest::rstest]
fn event_msg_type_name_is_module_scoped() {
    // Given a struct with #[derive(EventMsg)] and #[event_msg("test_mod")].
    // When accessing TYPE_NAME.
    // Then the value is "test_mod::TestEvent".
    assert_eq!(TestEvent::TYPE_NAME, "test_mod::TestEvent");
}

#[rstest::rstest]
fn command_msg_name_is_module_scoped() {
    // Given a struct with #[derive(CommandMsg)] and #[cmd("test_mod")].
    // When accessing NAME.
    // Then the value is "test_mod::TestCommand".
    assert_eq!(TestCommand::NAME, "test_mod::TestCommand");
}

/// Test fixture: event in a different module scope.
#[derive(Debug, Clone, EventMsg)]
#[event_msg("chat_input")]
struct ChatEntrySubmitted;

/// Test fixture: command in a different module scope.
#[derive(Debug, Clone, CommandMsg)]
#[cmd("chat_input")]
struct InsertChar;

#[rstest::rstest]
fn event_msg_test_event_has_correct_prefix() {
    // Given a struct with #[derive(EventMsg)] and #[event_msg("test_mod")].
    // When accessing TYPE_NAME.
    // Then the value is "test_mod::TestEvent".
    assert_eq!(TestEvent::TYPE_NAME, "test_mod::TestEvent");
}

#[rstest::rstest]
fn event_msg_chat_entry_submitted_has_correct_prefix() {
    // Given a struct with #[derive(EventMsg)] and #[event_msg("chat_input")].
    // When accessing TYPE_NAME.
    // Then the value is "chat_input::ChatEntrySubmitted".
    assert_eq!(
        ChatEntrySubmitted::TYPE_NAME,
        "chat_input::ChatEntrySubmitted"
    );
}

#[rstest::rstest]
fn command_msg_insert_char_has_correct_prefix() {
    // Given a struct with #[derive(CommandMsg)] and #[cmd("chat_input")].
    // When accessing NAME.
    // Then the value is "chat_input::InsertChar".
    assert_eq!(InsertChar::NAME, "chat_input::InsertChar");
}

#[rstest::rstest]
fn command_msg_test_command_has_correct_prefix() {
    // Given a struct with #[derive(CommandMsg)] and #[cmd("test_mod")].
    // When accessing NAME.
    // Then the value is "test_mod::TestCommand".
    assert_eq!(TestCommand::NAME, "test_mod::TestCommand");
}
