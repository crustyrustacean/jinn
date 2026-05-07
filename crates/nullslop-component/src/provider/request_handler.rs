//! Message queue — enqueues, dispatches, and drains user messages to the LLM.
//!
//! When a user submits a message, it goes through this handler. If the session
//! is idle, the message is dispatched immediately to the LLM. If the session
//! is busy (sending or streaming), the message is enqueued for later dispatch.
//!
//! On normal stream completion, all queued messages are dispatched at once in a single LLM call.
//! On cancel, the queue is drained and all messages are concatenated back into
//! the input box so the user doesn't lose their text.
//!
//! Session persistence is triggered by emitting [`SessionSaveRequested`] events
//! after immediate dispatch and after stream completion with `Finished` reason.

use std::collections::HashMap;

use npr::chat_input::{EnqueueUserMessage, SetChatInputText};
use npr::context::AssemblePrompt;
use npr::provider::{CancelStream, StreamCompleted, StreamCompletedReason};
use npr::session::SessionSaveRequested;
use nullslop_component_core::{HandlerContext, define_handler};
use nullslop_protocol as npr;
use nullslop_protocol::{ChatEntryKind, CommandAction};
use nullslop_services::Services;
use unicode_segmentation::UnicodeSegmentation as _;

use crate::AppState;

define_handler! {
    pub(crate) struct MessageQueueHandler;

    commands {
        EnqueueUserMessage: on_enqueue_user_message,
        CancelStream: on_cancel_stream,
        SetChatInputText: on_set_chat_input_text,
    }

    events {
        StreamCompleted: on_stream_completed,
    }
}

impl MessageQueueHandler {
    /// Enqueues a user message, dispatching immediately if idle or queuing if busy.
    ///
    /// `$name` tokens in the message text are expanded to template bodies
    /// before the text reaches chat history or the queue.
    fn on_enqueue_user_message(
        cmd: &EnqueueUserMessage,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        // Expand $name tokens before dispatching.
        let text =
            crate::prompt_template::expand_tokens(&cmd.text, &ctx.state.prompt_templates);

        let session = ctx.state.session_mut(&cmd.session_id);

        if session.is_idle() {
            // Dispatch immediately: push to history, request prompt assembly.
            let entry = npr::ChatEntry::user(&text);
            session.push_entry(entry);
            let history = session.history().to_vec();
            session.begin_assembling();

            ctx.out.submit_command(npr::Command::AssemblePrompt {
                payload: AssemblePrompt {
                    session_id: cmd.session_id.clone(),
                    history,
                    tools: vec![],
                    model_name: String::new(),
                },
            });

            // Request session persistence.
            emit_save_requested(ctx, &cmd.session_id);
        } else {
            // Session is busy — enqueue for later.
            session.enqueue_message(text);
        }

        CommandAction::Continue
    }

    /// Cancels the active stream and restores queued messages to the input box.
    fn on_cancel_stream(
        cmd: &CancelStream,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        let session = ctx.state.session_mut(&cmd.session_id);
        session.cancel_streaming();

        let drained: Vec<String> = session.drain_queue().into_iter().collect();
        if !drained.is_empty() {
            let restored = drained.join("\n");
            ctx.out.submit_command(npr::Command::SetChatInputText {
                payload: SetChatInputText {
                    session_id: cmd.session_id.clone(),
                    text: restored,
                },
            });
        }

        CommandAction::Continue
    }

    /// Replaces the chat input text for a session.
    fn on_set_chat_input_text(
        cmd: &SetChatInputText,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        let session = ctx.state.session_mut(&cmd.session_id);
        session.chat_input_mut().replace_all(cmd.text.clone());
        CommandAction::Continue
    }

    /// Handles stream completion, dispatching all queued messages at once if any.
    fn on_stream_completed(
        evt: &StreamCompleted,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) {
        let session = ctx.state.session_mut(&evt.session_id);
        // finish_streaming clears both is_streaming and is_sending.
        session.finish_streaming();

        // Only save on normal completion.
        // On cancel, the queue was already drained by on_cancel_stream.
        if evt.reason != StreamCompletedReason::Finished {
            return;
        }

        let drained: Vec<String> = session.drain_queue().into_iter().collect();
        if !drained.is_empty() {
            // Push all queued messages as individual user entries.
            for text in &drained {
                session.push_entry(npr::ChatEntry::user(text));
            }

            // Request prompt assembly instead of converting directly.
            let history = session.history().to_vec();
            session.begin_assembling();

            ctx.out.submit_command(npr::Command::AssemblePrompt {
                payload: AssemblePrompt {
                    session_id: evt.session_id.clone(),
                    history,
                    tools: vec![],
                    model_name: String::new(),
                },
            });
        }

        // Request session persistence after stream completion.
        emit_save_requested(ctx, &evt.session_id);
    }
}

/// Builds a [`SessionSaveRequested`] event from the session state and submits it.
///
/// This is the sync-side counterpart of the `session_to_persisted()` conversion —
/// it extracts the same fields but puts them into the event struct instead.
/// The actor receives the event and constructs a `PersistedSession` for disk writes.
fn emit_save_requested(
    ctx: &mut HandlerContext<'_, AppState, Services>,
    session_id: &npr::SessionId,
) {
    let session = ctx.state.session(session_id);
    let title = derive_title(session);

    let mut blobs = HashMap::new();
    if let Some(workflow) = session.workflow()
        && let Ok(value) = serde_json::to_value(workflow)
    {
        blobs.insert("workflow_state".to_owned(), value);
    }
    if let Some(strategy_state) = session.strategy_state() {
        blobs.insert("strategy_state".to_owned(), strategy_state.clone());
    }

    ctx.out.submit_event(npr::Event::SessionSaveRequested {
        payload: SessionSaveRequested {
            session_id: session_id.clone(),
            title,
            history: session.history().to_vec(),
            active_strategy: session.active_strategy().clone(),
            blobs,
        },
    });
}

/// Derives a session title from the first user message in the history.
///
/// Truncates to approximately 80 characters at a grapheme boundary using
/// [`unicode_segmentation`]. Returns `"New Session"` if no user messages exist.
fn derive_title(session: &crate::ChatSessionState) -> String {
    for entry in session.history() {
        if let ChatEntryKind::User(ref text) = entry.kind {
            return truncate_to_grapheme_boundary(text, 80);
        }
    }
    "New Session".to_owned()
}

/// Truncates a string to approximately `max_len` characters at a grapheme boundary.
///
/// If the string fits within `max_len` characters, it is returned unchanged.
/// If truncated, an ellipsis (`…`) is appended to indicate the truncation.
fn truncate_to_grapheme_boundary(text: &str, max_len: usize) -> String {
    if text.len() <= max_len {
        return text.to_owned();
    }

    let graphemes: Vec<&str> = text.graphemes(true).collect();
    let mut result = String::with_capacity(max_len + 1);
    let mut char_count = 0;

    for grapheme in &graphemes {
        let next_len = char_count + grapheme.len();
        if next_len > max_len {
            break;
        }
        result.push_str(grapheme);
        char_count = next_len;
    }

    result.push('…');
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt_template::{PromptTemplate, PromptTemplateStore};
    use crate::test_utils;
    use nullslop_component_core::Bus;
    use nullslop_protocol::Command;
    use nullslop_protocol::provider::StreamCompletedReason;
    use nullslop_services::Services;
    use nullslop_services::test_services::TestServices;

    /// Creates a test bus with `MessageQueueHandler` registered.
    fn test_bus() -> Bus<crate::AppState, Services> {
        let mut bus: Bus<crate::AppState, Services> = Bus::new();
        super::MessageQueueHandler.register(&mut bus);
        bus
    }

    /// Creates test services.
    fn test_services() -> Services {
        TestServices::builder().build()
    }

    #[test]
    fn submit_user_message_emits_session_save_requested() {
        // Given a bus with MessageQueueHandler registered.
        let mut bus = test_bus();
        let services = test_services();
        let mut state = crate::AppState::default();
        let session_id = state.active_session.clone();

        // When processing an EnqueueUserMessage command.
        bus.submit_command(npr::Command::EnqueueUserMessage {
            payload: EnqueueUserMessage {
                session_id: session_id.clone(),
                text: "hello world".to_owned(),
            },
        });
        bus.process_commands(&mut state, &services);

        // Events emitted during command processing need process_events to dispatch.
        bus.process_events(&mut state, &services);

        // Then a SessionSaveRequested event is emitted with matching session_id
        // and history containing the user message.
        let events = bus.drain_processed_events();
        let save_event = events.iter().find_map(|e| match &e.event {
            npr::Event::SessionSaveRequested { payload } => Some(payload.clone()),
            _ => None,
        });
        assert!(save_event.is_some(), "expected SessionSaveRequested event");
        let save_event = save_event.expect("should have save event");
        assert_eq!(save_event.session_id, session_id);
        assert_eq!(save_event.history.len(), 1);
        assert_eq!(save_event.title, "hello world");
    }

    #[test]
    fn stream_completed_with_finished_emits_session_save_requested() {
        // Given a bus with MessageQueueHandler registered, and a session with
        // a user message that's currently streaming.
        let mut bus = test_bus();
        let services = test_services();
        let mut state = crate::AppState::default();
        let session_id = state.active_session.clone();

        // Set up: push a user message and mark as sending.
        let session = state.session_mut(&session_id);
        session.push_entry(npr::ChatEntry::user("hello"));
        session.begin_sending();
        session.begin_streaming();

        // When processing StreamCompleted with Finished reason.
        bus.submit_event(npr::Event::StreamCompleted {
            payload: npr::provider::StreamCompleted {
                session_id: session_id.clone(),
                reason: StreamCompletedReason::Finished,
                assistant_content: None,
                tool_calls: None,
            },
        });
        bus.process_events(&mut state, &services);

        // Events emitted during event processing need another process_events call.
        bus.process_events(&mut state, &services);

        // Then a SessionSaveRequested event is emitted.
        let events = bus.drain_processed_events();
        let save_event = events.iter().find_map(|e| match &e.event {
            npr::Event::SessionSaveRequested { payload } => Some(payload),
            _ => None,
        });
        assert!(save_event.is_some(), "expected SessionSaveRequested event");
        let save_event = save_event.expect("should have save event");
        assert_eq!(save_event.session_id, session_id);
    }

    #[test]
    fn stream_completed_with_cancel_does_not_emit_save() {
        // Given a bus with MessageQueueHandler registered, and a streaming session.
        let mut bus = test_bus();
        let services = test_services();
        let mut state = crate::AppState::default();
        let session_id = state.active_session.clone();

        let session = state.session_mut(&session_id);
        session.push_entry(npr::ChatEntry::user("hello"));
        session.begin_sending();
        session.begin_streaming();

        // When processing StreamCompleted with Canceled reason.
        bus.submit_event(npr::Event::StreamCompleted {
            payload: npr::provider::StreamCompleted {
                session_id: session_id.clone(),
                reason: StreamCompletedReason::Canceled,
                assistant_content: None,
                tool_calls: None,
            },
        });
        bus.process_events(&mut state, &services);

        // Then NO SessionSaveRequested event is emitted.
        let events = bus.drain_processed_events();
        let has_save = events
            .iter()
            .any(|e| matches!(&e.event, npr::Event::SessionSaveRequested { .. }));
        assert!(!has_save, "should not emit SessionSaveRequested on cancel");
    }

    #[test]
    fn derive_title_returns_first_user_message_truncated() {
        // Given a session with a long first user message (> 80 chars).
        let mut bus = test_bus();
        let services = test_services();
        let mut state = crate::AppState::default();
        let session_id = state.active_session.clone();
        let long_message = "a".repeat(200);

        // When processing an EnqueueUserMessage with a long message.
        bus.submit_command(npr::Command::EnqueueUserMessage {
            payload: EnqueueUserMessage {
                session_id: session_id.clone(),
                text: long_message,
            },
        });
        bus.process_commands(&mut state, &services);
        bus.process_events(&mut state, &services);

        // Then the SessionSaveRequested event has a truncated title.
        let events = bus.drain_processed_events();
        let save_event = events.iter().find_map(|e| match &e.event {
            npr::Event::SessionSaveRequested { payload } => Some(payload.clone()),
            _ => None,
        });
        let save_event = save_event.expect("should have save event");
        assert!(
            save_event.title.len() <= 83,
            "title should be truncated to ~80 chars plus ellipsis"
        );
        assert!(
            save_event.title.ends_with('…'),
            "truncated title should end with ellipsis"
        );
    }

    #[test]
    fn derive_title_returns_default_when_no_messages() {
        // Given a session with no user messages (empty history).
        let session = crate::ChatSessionState::new();

        // When deriving the title.
        let title = derive_title(&session);

        // Then the title is "New Session".
        assert_eq!(title, "New Session");
    }

    #[test]
    fn derive_title_uses_first_user_message_not_assistant() {
        // Given a session with an assistant message before the user message.
        let mut session = crate::ChatSessionState::new();
        session.push_entry(npr::ChatEntry::assistant("I am a helper"));
        session.push_entry(npr::ChatEntry::user("my real question"));

        // When deriving the title.
        let title = derive_title(&session);

        // Then the title is the first user message, not the assistant message.
        assert_eq!(title, "my real question");
    }

    #[test]
    fn truncate_preserves_short_strings() {
        // Given a string shorter than max_len.
        // When truncating.
        let result = truncate_to_grapheme_boundary("hello", 80);
        // Then the string is unchanged.
        assert_eq!(result, "hello");
    }

    #[test]
    fn truncate_handles_multibyte_graphemes() {
        // Given a string with multi-byte characters.
        let emoji_string = "\u{1F389}".repeat(200);
        // When truncating.
        let result = truncate_to_grapheme_boundary(&emoji_string, 80);
        // Then the result is truncated and ends with ellipsis.
        assert!(result.ends_with('\u{2026}'), "should end with ellipsis");
        // Each emoji is 4 bytes, so at most 20 graphemes fit in 80 bytes + 3 for ellipsis.
        assert!(
            result.len() <= 83,
            "result should fit in 80 bytes of content + ellipsis"
        );
    }

    #[test]
    fn enqueue_user_message_expands_tokens() {
        // Given a bus with MessageQueueHandler registered and a store with a template.
        let mut bus: Bus<AppState, Services> = Bus::new();
        MessageQueueHandler.register(&mut bus);

        let store = PromptTemplateStore::from_vec(vec![PromptTemplate {
            name: "code-review".to_owned(),
            description: "Review".to_owned(),
            body: "Review this code.".to_owned(),
        }]);

        let mut state = AppState {
            prompt_templates: store,
            ..AppState::default()
        };
        let session_id = state.active_session.clone();

        // When enqueueing a message with a $name token.
        bus.submit_command(Command::EnqueueUserMessage {
            payload: EnqueueUserMessage {
                session_id: session_id.clone(),
                text: "$code-review".to_owned(),
            },
        });
        let services = test_utils::test_services();
        bus.process_commands(&mut state, &services);

        // Then the chat history contains the expanded template body.
        let session = state.session(&session_id);
        let history = session.history();
        assert_eq!(history.len(), 1);
        assert_eq!(
            &history[0].kind,
            &npr::ChatEntryKind::User("Review this code.".to_owned())
        );
    }
}
