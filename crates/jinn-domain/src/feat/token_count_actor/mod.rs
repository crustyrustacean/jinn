//! Token count actor — computes tiktoken-based counts for chat entries.
//!
//! Subscribes to [`HistoryAppended`] and [`SessionLoadCompleted`] to
//! asynchronously compute per-entry token counts and write them to the
//! [`EntryTokenCache`] in `FrontendCaches`. The minimap render pipeline
//! reads the cache synchronously during rendering.

use kameo::actor::ActorRef;
use kameo::prelude::{Context, Message};

use crate::common::actor_deps::ActorDeps;
use crate::common::state::State;
use crate::feat::context::protocol::event::ContextOverrideChanged;
use crate::feat::context::strategy::token_estimator::{
    TiktokenCounter, TokenCounter, TokenEstimator, estimate_entry_tokens,
};
use crate::feat::session::protocol::history_appended::HistoryAppended;
use crate::feat::session::protocol::session_load_completed::SessionLoadCompleted;

/// Dependencies for [`TokenCountActor`].
#[derive(Clone)]
pub struct TokenCountActorDeps {
    /// Common actor dependencies (services + bus).
    pub deps: ActorDeps,
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

impl kameo::Actor for TokenCountActor {
    type Args = TokenCountActorDeps;
    type Error = kameo::error::Infallible;

    async fn on_start(args: Self::Args, actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        args.deps
            .subscribe(actor_ref.clone().recipient::<HistoryAppended>())
            .await;
        args.deps
            .subscribe(actor_ref.recipient::<SessionLoadCompleted>())
            .await;

        Ok(Self {
            state: args.state,
            counter: TiktokenCounter::o200k_base(),
        })
    }
}

impl Message<HistoryAppended> for TokenCountActor {
    type Reply = ();

    async fn handle(&mut self, msg: HistoryAppended, _ctx: &mut Context<Self, Self::Reply>) {
        self.handle_history_appended(&msg.session_id);
    }
}

impl Message<SessionLoadCompleted> for TokenCountActor {
    type Reply = ();

    async fn handle(&mut self, msg: SessionLoadCompleted, _ctx: &mut Context<Self, Self::Reply>) {
        self.handle_session_load_completed(&msg.session);
    }
}

impl Message<ContextOverrideChanged> for TokenCountActor {
    type Reply = ();

    async fn handle(&mut self, msg: ContextOverrideChanged, _ctx: &mut Context<Self, Self::Reply>) {
        self.handle_context_override_changed(&msg.session_id, &msg.entry_id);
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

    /// Handle a [`ContextOverrideChanged`] event.
    ///
    /// Recomputes the token count for the entry if it is now in context
    /// but the cached value is 0 or missing. This handles the case where
    /// an entry was excluded by a pruner, cached at 0, then re-inserted
    /// by the user.
    fn handle_context_override_changed(
        &self,
        session_id: &crate::protocol::SessionId,
        entry_id: &crate::protocol::ChatEntryId,
    ) {
        let should_recompute = {
            let state = self.state.read();
            let Some(session) = state.try_session(session_id) else {
                return;
            };
            let Some(idx) = session.find_entry_index_by_id(entry_id) else {
                return;
            };
            let Some(entry) = session.history().get(idx) else {
                return;
            };

            if !entry.is_in_context() {
                return;
            }

            let cache = state.frontend.caches.entry_token_cache.read();
            matches!(cache.get(&entry.id), None | Some(0))
        };

        if !should_recompute {
            return;
        }

        let tokens = {
            let state = self.state.read();
            let Some(session) = state.try_session(session_id) else {
                return;
            };
            let Some(idx) = session.find_entry_index_by_id(entry_id) else {
                return;
            };
            let Some(entry) = session.history().get(idx) else {
                return;
            };
            let estimator = TiktokenEstimator(self.counter);
            estimate_entry_tokens(&estimator, entry) as u32
        };

        if tokens > 0 {
            let state = self.state.write();
            state
                .frontend
                .caches
                .entry_token_cache
                .write()
                .insert(entry_id.clone(), tokens);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use super::*;
    use crate::common::app_state::AppState;
    use crate::feat::session::chat_entry::{ChatEntry, ChatEntryKind, ContextOverride};

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

    #[rstest::rstest]
    fn context_override_changed_recomputes_stale_zero_count() {
        // Given a state with one user entry that was excluded and cached at 0.
        let mut app_state = AppState::default();
        app_state
            .active_session_mut()
            .push_entry(ChatEntry::user("hello world"));
        let entry_id = app_state.active_session().history()[0].id.clone();
        app_state
            .frontend
            .caches
            .entry_token_cache
            .write()
            .insert(entry_id.clone(), 0);

        let state = State::new(app_state);
        let actor = TokenCountActor {
            state: state.clone(),
            counter: TiktokenCounter::o200k_base(),
        };

        // When handling ContextOverrideChanged.
        let session_id = state.read().session.active_session_id().clone();
        actor.handle_context_override_changed(&session_id, &entry_id);

        // Then the cache now has a nonzero count.
        let state_guard = state.read();
        let cache = state_guard.frontend.caches.entry_token_cache.read();
        let count = cache.get(&entry_id);
        assert_eq!(count, Some(2));
    }

    #[rstest::rstest]
    fn context_override_changed_noop_for_nonzero_cache() {
        // Given a state with one user entry already cached at 500.
        let mut app_state = AppState::default();
        app_state
            .active_session_mut()
            .push_entry(ChatEntry::user("hello world"));
        let entry_id = app_state.active_session().history()[0].id.clone();
        app_state
            .frontend
            .caches
            .entry_token_cache
            .write()
            .insert(entry_id.clone(), 500);

        let state = State::new(app_state);
        let actor = TokenCountActor {
            state: state.clone(),
            counter: TiktokenCounter::o200k_base(),
        };

        // When handling ContextOverrideChanged.
        let session_id = state.read().session.active_session_id().clone();
        actor.handle_context_override_changed(&session_id, &entry_id);

        // Then the cached count is unchanged.
        let state_guard = state.read();
        let cache = state_guard.frontend.caches.entry_token_cache.read();
        assert_eq!(cache.get(&entry_id), Some(500));
    }

    #[rstest::rstest]
    fn context_override_changed_noop_when_excluded() {
        // Given a state with one user entry that is ForcedExclude.
        let mut app_state = AppState::default();
        let mut entry = ChatEntry::user("hello world");
        entry.context_override = ContextOverride::ForcedExclude;
        app_state.active_session_mut().push_entry(entry);
        let entry_id = app_state.active_session().history()[0].id.clone();
        // No cache entry — should still be noop because entry is excluded.

        let state = State::new(app_state);
        let actor = TokenCountActor {
            state: state.clone(),
            counter: TiktokenCounter::o200k_base(),
        };

        // When handling ContextOverrideChanged.
        let session_id = state.read().session.active_session_id().clone();
        actor.handle_context_override_changed(&session_id, &entry_id);

        // Then no cache entry was created.
        let state_guard = state.read();
        let cache = state_guard.frontend.caches.entry_token_cache.read();
        assert_eq!(cache.get(&entry_id), None);
    }

    #[rstest::rstest]
    fn context_override_changed_recomputes_missing_cache_entry() {
        // Given a state with one user entry that has no cache entry.
        let mut app_state = AppState::default();
        app_state
            .active_session_mut()
            .push_entry(ChatEntry::user("hello world"));
        let entry_id = app_state.active_session().history()[0].id.clone();
        // Entry is in context (Default) but has never been cached.

        let state = State::new(app_state);
        let actor = TokenCountActor {
            state: state.clone(),
            counter: TiktokenCounter::o200k_base(),
        };

        // When handling ContextOverrideChanged.
        let session_id = state.read().session.active_session_id().clone();
        actor.handle_context_override_changed(&session_id, &entry_id);

        // Then the cache now has a nonzero count.
        let state_guard = state.read();
        let cache = state_guard.frontend.caches.entry_token_cache.read();
        let count = cache.get(&entry_id);
        assert_eq!(count, Some(2));
    }

    #[rstest::rstest]
    fn large_tool_call_arguments_counted_correctly() {
        // Given a tool call entry with large arguments (simulating write tool).
        let large_content = "fn main() { println!(\"hello\"); }\n".repeat(200);
        let arguments = format!(r#"{{\"path\":\"test.rs\",\"content\":\"{large_content}\"}}"#);
        let counter = TiktokenCounter::o200k_base();
        let direct_count = counter.count(&arguments);

        let mut app_state = AppState::default();
        app_state
            .active_session_mut()
            .push_entry(ChatEntry::tool_call("call_1", "write", &arguments));

        // Verify the entry actually has the full arguments.
        let entry = &app_state.active_session().history()[0];
        if let ChatEntryKind::ToolCall {
            arguments: ref args,
            ..
        } = entry.kind
        {
            assert_eq!(args.len(), arguments.len());
        }

        let state = State::new(app_state);
        let actor = TokenCountActor {
            state: state.clone(),
            counter: TiktokenCounter::o200k_base(),
        };

        // When handling HistoryAppended.
        let session_id = state.read().session.active_session_id().clone();
        actor.handle_history_appended(&session_id);

        // Then the cached count should be close to the direct count.
        let entry_id = state.read().active_session().history()[0].id.clone();
        let state_guard = state.read();
        let cache = state_guard.frontend.caches.entry_token_cache.read();
        let count = cache.get(&entry_id).expect("entry should be cached");
        let tool_name_tokens = counter.count("write");
        let expected_min = direct_count + tool_name_tokens;
        assert!(
            count as usize > expected_min / 2,
            "expected count > {} (half of {}), got {}",
            expected_min / 2,
            expected_min,
            count
        );
        assert!(
            count as usize > 500,
            "expected count > 500 for large tool call, got {count}"
        );
    }
}
