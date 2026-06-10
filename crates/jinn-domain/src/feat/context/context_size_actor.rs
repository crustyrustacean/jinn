//! Context size actor - recalculates active session context size after changes.
//!
//! Subscribes to events that affect context size and runs `assemble_prompt()`
//! to update `cached_context_size` for the active session. Uses eager
//! recalculation — each event triggers an immediate assembly.

use crate::common::actor::{Actor, ActorContext, ActorEnvelope, NoDirectMsg};
use crate::common::state::State;
use crate::feat::context::assemble::assemble_prompt;
use crate::feat::context::strategy::token_estimator::TiktokenCounter;
use crate::protocol::Event;
use tracing::error;

/// Recalculates context size for the active session after context-affecting changes.
///
/// Subscribes to events that change what's included in the assembled prompt
/// (history additions, context overrides, pin changes, session switches)
/// and updates `cached_context_size` so the status bar stays accurate.
pub struct ContextSizeActor {
    /// Shared application state.
    state: State,
    /// Token counter for prompt assembly.
    counter: TiktokenCounter,
}

/// Dependencies for [`ContextSizeActor`].
pub struct ContextSizeActorDeps {
    /// Shared application state.
    pub state: State,
    /// Token counter for prompt assembly.
    pub counter: TiktokenCounter,
}

impl Actor for ContextSizeActor {
    type Message = NoDirectMsg;
    type Deps = ContextSizeActorDeps;

    fn activate(deps: Self::Deps, ctx: &mut ActorContext) -> Self {
        // Subscribe to all events that can change context size.
        ctx.subscribe_event::<crate::feat::session::protocol::history_appended::HistoryAppended>();
        ctx.subscribe_event::<crate::feat::context::protocol::event::ContextOverrideChanged>();
        ctx.subscribe_event::<crate::protocol::system::ActiveSessionChanged>();
        ctx.subscribe_event::<crate::feat::context::protocol::event::ChatEntryPinChanged>();
        ctx.subscribe_event::<crate::feat::session::protocol::session_load_completed::SessionLoadCompleted>();

        ctx.set_description("Context size recalculation for status bar");

        Self {
            state: deps.state,
            counter: deps.counter,
        }
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, _ctx: &ActorContext) {
        if let ActorEnvelope::Event(event) = &msg
            && Self::is_context_relevant(event)
        {
            self.recalculate().await;
        }
    }
}

impl ContextSizeActor {
    /// Check if this event is relevant to context size.
    fn is_context_relevant(event: &Event) -> bool {
        matches!(
            event,
            Event::HistoryAppended(_)
                | Event::ContextOverrideChanged(_)
                | Event::ActiveSessionChanged(_)
                | Event::ChatEntryPinChanged(_)
                | Event::SessionLoadCompleted(_)
        )
    }

    /// Recalculate context size for the active session.
    ///
    /// The CPU-intensive `assemble_prompt` call is moved into
    /// `tokio::task::spawn_blocking` to avoid consuming the async worker's
    /// coop budget during startup bursts.
    async fn recalculate(&self) {
        let session_id = {
            let state = self.state.read();
            state.session.active_session_id().clone()
        };

        let state_clone = self.state.clone();
        let counter = self.counter;
        let id_for_blocking = session_id.clone();
        let result = tokio::task::spawn_blocking(move || {
            let guard = state_clone.read();
            assemble_prompt(&guard, &id_for_blocking, &counter, None).estimated_tokens()
        })
        .await;

        match result {
            Ok(assembled_tokens) => {
                let mut state = self.state.write();
                if let Some(session) = state.session.get_mut(&session_id) {
                    session.set_context_size(assembled_tokens);
                }
            }
            Err(join_err) => {
                error!(error = %join_err, "context-size recalculate task failed");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unreachable, clippy::indexing_slicing, reason = "test code")]

    use super::*;
    use crate::common::app_state::AppState;
    use crate::common::state::State;
    use crate::feat::session::chat_session::ChatSessionState;
    use crate::protocol::{ChatEntry, SessionId};

    fn test_actor() -> ContextSizeActor {
        ContextSizeActor {
            state: State::new(AppState::default()),
            counter: TiktokenCounter::o200k_base(),
        }
    }

    #[tokio::test]
    async fn recalculate_updates_context_size_for_active_session() {
        // Given an actor with a session that has history.
        let actor = test_actor();
        let session_id = {
            let mut state = actor.state.write();
            state
                .active_session_mut()
                .push_entry(ChatEntry::user("hello world"));
            state.session.active_session_id().clone()
        };

        // When recalculating.
        actor.recalculate().await;

        // Then context_size is set to a positive value.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session");
        let ctx_size = session.context_size().expect("context size should be set");
        assert!(
            ctx_size > 0,
            "context_size should be positive, got {ctx_size}"
        );
    }

    #[tokio::test]
    async fn recalculate_updates_after_entry_added() {
        // Given an actor with empty session.
        let actor = test_actor();
        let session_id = {
            let state = actor.state.read();
            state.session.active_session_id().clone()
        };

        // Recalculate with empty history.
        actor.recalculate().await;
        let size_before = {
            let state = actor.state.read();
            state
                .session
                .get(&session_id)
                .expect("session")
                .context_size()
                .expect("should be set")
        };

        // When adding an entry and recalculating.
        {
            let mut state = actor.state.write();
            state
                .active_session_mut()
                .push_entry(ChatEntry::user("a long message that adds tokens"));
        }
        actor.recalculate().await;

        // Then context_size increased.
        let size_after = {
            let state = actor.state.read();
            state
                .session
                .get(&session_id)
                .expect("session")
                .context_size()
                .expect("should be set")
        };
        assert!(
            size_after > size_before,
            "context_size should increase after adding entry: {size_after} vs {size_before}"
        );
    }

    #[tokio::test]
    async fn recalculate_handles_empty_session_gracefully() {
        // Given an actor with empty session.
        let actor = test_actor();

        // When recalculating.
        actor.recalculate().await;

        // Then context_size is set (system prompt only).
        let state = actor.state.read();
        let ctx_size = state
            .active_session()
            .context_size()
            .expect("should be set even for empty session");
        // Should have some tokens from the system prompt / env context.
        assert!(
            ctx_size > 0,
            "context_size should be positive even for empty session, got {ctx_size}"
        );
    }

    #[tokio::test]
    async fn recalculate_only_updates_active_session() {
        // Given an actor with two sessions.
        let actor = test_actor();
        let second = ChatSessionState::new();
        let second_id = second.session_id().clone();
        {
            let mut state = actor.state.write();
            state.session.insert(second);
            // Active session has history, second does not.
            state
                .active_session_mut()
                .push_entry(ChatEntry::user("hello"));
        }

        // When recalculating.
        actor.recalculate().await;

        // Then active session has context_size set, second does not.
        let state = actor.state.read();
        assert!(
            state.active_session().context_size().is_some(),
            "active session should have context_size"
        );
        assert!(
            state
                .session
                .get(&second_id)
                .expect("second")
                .context_size()
                .is_none(),
            "non-active session should not have context_size set"
        );
    }

    #[tokio::test]
    async fn is_context_relevant_matches_expected_events() {
        // Given various events.
        use crate::feat::session::chat_entry::ChatEntryId;

        assert!(
            ContextSizeActor::is_context_relevant(&Event::HistoryAppended(
                crate::feat::session::protocol::history_appended::HistoryAppended {
                    session_id: SessionId::new(),
                }
            )),
            "HistoryAppended should be relevant"
        );

        assert!(
            ContextSizeActor::is_context_relevant(&Event::ContextOverrideChanged(
                crate::feat::context::protocol::event::ContextOverrideChanged {
                    session_id: SessionId::new(),
                    entry_id: ChatEntryId::new(),
                }
            )),
            "ContextOverrideChanged should be relevant"
        );

        assert!(
            ContextSizeActor::is_context_relevant(&Event::ActiveSessionChanged(
                crate::protocol::system::ActiveSessionChanged {
                    session_id: SessionId::new(),
                }
            )),
            "ActiveSessionChanged should be relevant"
        );

        assert!(
            ContextSizeActor::is_context_relevant(&Event::ChatEntryPinChanged(
                crate::feat::context::protocol::event::ChatEntryPinChanged {
                    session_id: SessionId::new(),
                }
            )),
            "ChatEntryPinChanged should be relevant"
        );
    }

    #[tokio::test]
    async fn is_context_relevant_ignores_irrelevant_events() {
        // Given an irrelevant event.
        assert!(
            !ContextSizeActor::is_context_relevant(&Event::SessionCreated(
                crate::feat::session_lifecycle::protocol::event::SessionCreated {
                    session_id: SessionId::new(),
                }
            )),
            "SessionCreated should not be relevant"
        );
    }
}
