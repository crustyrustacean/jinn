//! Token count actor - computes tiktoken-based counts for chat entries.
//!
//! Subscribes to [`HistoryAppended`] and [`SessionLoadCompleted`] to
//! asynchronously compute per-entry token counts and write them to the
//! [`EntryTokenCache`] in `FrontendCaches`. The minimap render pipeline
//! reads the cache synchronously during rendering.

use crate::common::actor::scan_actor::NoDirectMsg;
use crate::common::actor::{Actor, ActorContext, ActorEnvelope};
use crate::common::state::State;
use crate::feat::context::strategy::token_estimator::{
    TokenCounter, TokenEstimator, TiktokenCounter, estimate_entry_tokens,
};
use crate::feat::session::protocol::history_appended::HistoryAppended;
use crate::feat::session::protocol::session_load_completed::SessionLoadCompleted;
use crate::protocol::Event;

/// Dependencies for [`TokenCountActor`].
pub struct TokenCountActorDeps {
    /// Shared application state.
    pub state: State,
}

/// The token count actor.
///
/// Computes tiktoken-based token counts for chat entries and caches them
/// in `FrontendCaches::entry_token_cache`. Runs asynchronously so the
/// render pipeline is never blocked by tiktoken computation.
pub struct TokenCountActor {
    state: State,
    counter: TiktokenCounter,
}

/// Thin adapter that implements [`TokenEstimator`] by delegating to
/// [`TiktokenCounter::count()`]. This bridges the gap between the
/// `TokenCounter` trait (which `TiktokenCounter` implements) and the
/// `TokenEstimator` trait (which `estimate_entry_tokens` requires).
struct TiktokenEstimator(TiktokenCounter);

impl TokenEstimator for TiktokenEstimator {
    fn estimate(&self, text: &str) -> usize {
        self.0.count(text)
    }

    fn name(&self) -> &'static str {
        "tiktoken_adapter"
    }
}

impl Actor for TokenCountActor {
    type Message = NoDirectMsg;
    type Deps = TokenCountActorDeps;

    fn activate(deps: Self::Deps, ctx: &mut ActorContext) -> Self {
        ctx.set_description("Computes tiktoken-based token counts for chat entries");
        ctx.subscribe_event::<HistoryAppended>();
        ctx.subscribe_event::<SessionLoadCompleted>();

        Self {
            state: deps.state,
            counter: TiktokenCounter::o200k_base(),
        }
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, _ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Event(Event::HistoryAppended(ref payload)) => {
                self.handle_history_appended(&payload.session_id);
            }
            ActorEnvelope::Event(Event::SessionLoadCompleted(ref payload)) => {
                self.handle_session_load_completed(&payload.session);
            }
            ActorEnvelope::Command(_)
            | ActorEnvelope::Event(_)
            | ActorEnvelope::System(_)
            | ActorEnvelope::Direct(_) => {}
        }
    }
}

impl TokenCountActor {
    /// Handle a [`HistoryAppended`] event.
    ///
    /// Computes tiktoken counts for any entries in the active session's
    /// history that are not already in the cache.
    fn handle_history_appended(&self, _session_id: &crate::protocol::SessionId) {
        let new_counts = {
            let state = self.state.read();
            let session = state.active_session();
            let history = session.history();

            let cache = state.frontend.caches.entry_token_cache.read();
            let estimator = TiktokenEstimator(self.counter);

            let mut counts = Vec::new();
            for entry in history {
                if cache.contains(&entry.id) {
                    continue;
                }
                let tokens = estimate_entry_tokens(&estimator, entry);
                counts.push((entry.id.clone(), tokens as u32));
            }
            counts
        };

        if !new_counts.is_empty() {
            let state = self.state.write();
            state
                .frontend
                .caches
                .entry_token_cache
                .write()
                .bulk_insert(new_counts);
        }
    }

    /// Handle a [`SessionLoadCompleted`] command.
    ///
    /// Batch-computes tiktoken counts for all entries in the loaded session
    /// that are not already in the cache.
    fn handle_session_load_completed(&self, session: &crate::feat::session::ChatSessionState) {
        let new_counts = {
            let state = self.state.read();
            let cache = state.frontend.caches.entry_token_cache.read();
            let estimator = TiktokenEstimator(self.counter);

            let mut counts = Vec::new();
            for entry in session.history() {
                if cache.contains(&entry.id) {
                    continue;
                }
                let tokens = estimate_entry_tokens(&estimator, entry);
                counts.push((entry.id.clone(), tokens as u32));
            }
            counts
        };

        if !new_counts.is_empty() {
            let state = self.state.write();
            state
                .frontend
                .caches
                .entry_token_cache
                .write()
                .bulk_insert(new_counts);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use super::*;
    use crate::common::app_state::AppState;
    use crate::feat::session::chat_entry::ChatEntry;

    #[rstest::rstest]
    fn history_appended_computes_count_for_new_entry() {
        // Given a state with one user entry and an empty cache.
        let mut app_state = AppState::default();
        app_state
            .active_session_mut()
            .push_entry(ChatEntry::user("hello world"));
        let state = State::new(app_state);

        let actor = TokenCountActor {
            state: state.clone(),
            counter: TiktokenCounter::o200k_base(),
        };

        // When handling HistoryAppended.
        let session_id = state.read().session.active_session_id().clone();
        actor.handle_history_appended(&session_id);

        // Then the cache has a count for the entry.
        let entry_id = state.read().active_session().history()[0].id.clone();
        let state_guard = state.read();
        let cache = state_guard.frontend.caches.entry_token_cache.read();
        let count = cache.get(&entry_id);
        assert!(count.is_some(), "entry should have a cached token count");
        assert!(
            count.unwrap() > 0,
            "token count should be positive for non-empty text"
        );
    }

    #[rstest::rstest]
    fn history_appended_skips_already_cached() {
        // Given a state with one entry already in the cache.
        let mut app_state = AppState::default();
        app_state
            .active_session_mut()
            .push_entry(ChatEntry::user("hello world"));
        let entry_id = app_state.active_session().history()[0].id.clone();

        // Pre-populate the cache.
        app_state
            .frontend
            .caches
            .entry_token_cache
            .write()
            .insert(entry_id.clone(), 42);

        let state = State::new(app_state);
        let actor = TokenCountActor {
            state: state.clone(),
            counter: TiktokenCounter::o200k_base(),
        };

        // When handling HistoryAppended.
        let session_id = state.read().session.active_session_id().clone();
        actor.handle_history_appended(&session_id);

        // Then the cached count is unchanged (not re-computed).
        let state_guard = state.read();
        let cache = state_guard.frontend.caches.entry_token_cache.read();
        assert_eq!(cache.get(&entry_id), Some(42));
    }

    #[rstest::rstest]
    fn session_load_batch_computes_all_entries() {
        // Given a loaded session with 5 entries.
        let mut loaded_session = crate::feat::session::ChatSessionState::new();
        for i in 0..5 {
            loaded_session.push_entry(ChatEntry::user(format!("message {i}")));
        }

        let state = State::new(AppState::default());
        let actor = TokenCountActor {
            state: state.clone(),
            counter: TiktokenCounter::o200k_base(),
        };

        // When handling SessionLoadCompleted.
        actor.handle_session_load_completed(&loaded_session);

        // Then all 5 entries have cached counts.
        let state_guard = state.read();
        let cache = state_guard.frontend.caches.entry_token_cache.read();
        for entry in loaded_session.history() {
            assert!(
                cache.contains(&entry.id),
                "entry {:?} should be cached",
                entry.id
            );
        }
    }

    #[rstest::rstest]
    fn count_matches_tiktoken_for_known_text() {
        // Given a counter and a known text.
        let counter = TiktokenCounter::o200k_base();
        let text = "hello world";
        let expected = counter.count(text);

        // When computing via the adapter.
        let estimator = TiktokenEstimator(counter);
        let entry = ChatEntry::user(text);
        let actual = estimate_entry_tokens(&estimator, &entry);

        // Then the count matches direct tiktoken counting.
        assert_eq!(actual, expected);
    }
}
