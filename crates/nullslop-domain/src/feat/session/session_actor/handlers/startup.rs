//! Startup handler — applies config defaults on environment load.
//!
//! On startup, loads user preferences, applies model/strategy defaults to the
//! default session, loads unarchived sessions from SQLite, and emits commands
//! to initialize the context and preferences pipelines.

use crate::common::actor::ActorContext;
use crate::feat::context::protocol::command::SwitchPromptStrategy;
use crate::protocol::{Command, PromptStrategyId};

use super::super::SessionPersistenceActor;

impl SessionPersistenceActor {
    /// Applies config defaults to the default session profile on startup.
    ///
    /// Loads user preferences and applies `last_model` and `last_strategy`
    /// to the default session, then sends an `UpdatePreferences` command so
    /// the preferences pipeline handles persistence and state sync.
    ///
    /// NOTE: Using `active_session_mut()` is acceptable here because this runs
    /// at startup before any user interaction. There is only one session.
    pub(in crate::feat::session::session_actor) async fn on_environment_loaded(
        &self,
        _config: &crate::feat::provider_infra::ProvidersConfig,
        ctx: &ActorContext,
    ) {
        let Some(ref services) = self.services else {
            return;
        };

        let prefs = match services.user_preferences_storage.load() {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(err = ?e, "session-actor failed to load preferences on startup");
                return;
            }
        };

        {
            let mut state = self.state.write();

            // Apply config defaults to the default session.
            let session = state.active_session_mut();
            if let Some(ref model) = prefs.last_model {
                session.set_model(model.clone());
            }
            if let Some(ref strategy_str) = prefs.last_strategy {
                let strategy_id = PromptStrategyId::new(strategy_str.clone());
                session.switch_strategy(strategy_id.clone());
            }
        }

        // Load unarchived sessions from SQLite into memory.
        // These are sessions with `archived=false` — corresponding to `SessionState::Loaded`.
        // On load they get the default `SessionState::Loaded` and `LifecycleScriptState::NothingRan`.
        if let Some(ref store) = self.store {
            let summaries = match store.load_unarchived_summaries().await {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(err = ?e, "session-actor failed to load unarchived summaries on startup");
                    return;
                }
            };

            if !summaries.is_empty() {
                // Sort by updated_at descending to find the most recent.
                let mut sorted = summaries;
                sorted.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

                // Load full sessions outside the lock.
                let mut loaded = Vec::new();
                for summary in &sorted {
                    if let Ok(Some(session)) = store.load_session(&summary.session_id).await {
                        loaded.push(session);
                    }
                }

                if !loaded.is_empty() {
                    let mut state = self.state.write();

                    for session in loaded {
                        state
                            .session
                            .sessions_mut()
                            .insert(session.session_id().clone(), session);
                    }

                    // NOTE: We intentionally do NOT switch the active session.
                    // The user should land on the fresh welcome session.
                    // Previously loaded sessions appear in the sidebar but
                    // don't steal focus.
                }
            }
        }

        // Send UpdatePreferences command so the pipeline handles persistence + state sync.
        if let Err(e) = ctx.send_command(Command::UpdatePreferences(crate::feat::preferences_actor::protocol::command::UpdatePreferences {
                updates: vec![
                    crate::feat::preferences_actor::protocol::command::PreferenceUpdate::SetLastModel(prefs.last_model.clone()),
                    crate::feat::preferences_actor::protocol::command::PreferenceUpdate::SetLastStrategy(prefs.last_strategy.clone()),
                ],
            })) {
            tracing::warn!(err = ?e, "session-actor failed to send UpdatePreferences on startup");
        }

        // Emit SwitchPromptStrategy so the context actor initializes the strategy.
        if let Some(ref strategy_str) = prefs.last_strategy {
            // Get the current active session (may have changed after loading unarchived sessions).
            let active_id = self.state.read().session.active_session_id().clone();
            let strategy_id = PromptStrategyId::new(strategy_str.clone());
            if let Err(e) = ctx.send_command(Command::SwitchPromptStrategy(SwitchPromptStrategy {
                session_id: active_id,
                strategy_id,
            })) {
                tracing::error!(err = ?e, "failed to send command");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use super::super::super::helpers::{test_actor, test_context};
    use crate::common::app_state::AppState;
    use crate::common::state::State;
    use crate::feat::context::strategy::token_estimator::TiktokenCounter;
    use crate::feat::session::chat_session::ChatSessionState;
    use crate::feat::session::session_store::SessionStore;
    use std::sync::Arc;

    /// A fake session store that returns pre-loaded sessions for testing.
    struct PopulatedFakeStore {
        summaries: Vec<crate::feat::session::session_summary::SessionSummary>,
        sessions: Vec<ChatSessionState>,
    }

    impl PopulatedFakeStore {
        fn new(sessions: Vec<ChatSessionState>) -> Self {
            let summaries = sessions
                .iter()
                .map(|s| crate::feat::session::session_summary::SessionSummary {
                    session_id: s.session_id().clone(),
                    title: s.title().unwrap_or("Untitled Session").to_owned(),
                    updated_at: *s.updated_at(),
                    created_at: *s.created_at(),
                    session_state: crate::feat::session::chat_session::SessionState::Loaded,
                })
                .collect();
            Self {
                summaries,
                sessions,
            }
        }
    }

    #[async_trait::async_trait]
    impl SessionStore for PopulatedFakeStore {
        fn name(&self) -> &'static str {
            "populated-fake"
        }

        async fn save(
            &self,
            _session: &ChatSessionState,
        ) -> Result<(), error_stack::Report<crate::feat::session::session_store::SessionStoreError>>
        {
            Ok(())
        }

        async fn load_summaries(
            &self,
        ) -> Result<
            Vec<crate::feat::session::session_summary::SessionSummary>,
            error_stack::Report<crate::feat::session::session_store::SessionStoreError>,
        > {
            Ok(self.summaries.clone())
        }

        async fn load_session(
            &self,
            session_id: &crate::protocol::SessionId,
        ) -> Result<
            Option<ChatSessionState>,
            error_stack::Report<crate::feat::session::session_store::SessionStoreError>,
        > {
            Ok(self
                .sessions
                .iter()
                .find(|s| s.session_id() == session_id)
                .cloned())
        }

        async fn delete(
            &self,
            _session_id: &crate::protocol::SessionId,
        ) -> Result<(), error_stack::Report<crate::feat::session::session_store::SessionStoreError>>
        {
            Ok(())
        }

        async fn fork(
            &self,
            _source_session_id: &crate::protocol::SessionId,
            _at_ordinal: usize,
        ) -> Result<
            crate::protocol::SessionId,
            error_stack::Report<crate::feat::session::session_store::SessionStoreError>,
        > {
            Ok(crate::protocol::SessionId::new())
        }

        async fn set_archived(
            &self,
            _session_id: &crate::protocol::SessionId,
            _archived: bool,
        ) -> Result<(), error_stack::Report<crate::feat::session::session_store::SessionStoreError>>
        {
            Ok(())
        }

        async fn load_unarchived_summaries(
            &self,
        ) -> Result<
            Vec<crate::feat::session::session_summary::SessionSummary>,
            error_stack::Report<crate::feat::session::session_store::SessionStoreError>,
        > {
            Ok(self.summaries.clone())
        }
    }

    /// Builds a test actor with services and a populated store.
    fn test_actor_with_store(
        sessions: Vec<ChatSessionState>,
    ) -> super::super::SessionPersistenceActor {
        let services = crate::TestServices::builder()
            .session_store(super::super::SessionStoreService::new(Arc::new(PopulatedFakeStore::new(sessions))))
            .build();
        let store = services.session_store.clone();
        super::super::SessionPersistenceActor {
            state: State::new(AppState::default()),
            services: Some(services),
            store: Some(store),
            counter: TiktokenCounter::o200k_base(),
        }
    }

    #[tokio::test]
    async fn loading_unarchived_sessions_does_not_switch_active_session() {
        // Given an actor with a default welcome session and one session in the store.
        let store_session = ChatSessionState::new();
        let actor = test_actor_with_store(vec![store_session]);
        let (_sink, ctx) = test_context();

        // Record the default session's ID before loading.
        let default_id = actor.state.read().session.active_session_id().clone();

        // When handling EnvironmentLoaded.
        actor
            .on_environment_loaded(
                &crate::feat::provider_infra::ProvidersConfig {
                    providers: vec![],
                    aliases: vec![],
                    default_provider: None,
                },
                &ctx,
            )
            .await;

        // Then the active session is still the default.
        let state = actor.state.read();
        assert_eq!(*state.session.active_session_id(), default_id);
    }

    #[tokio::test]
    async fn loading_unarchived_sessions_does_not_remove_default_session() {
        // Given an actor with a default welcome session and one session in the store.
        let store_session = ChatSessionState::new();
        let actor = test_actor_with_store(vec![store_session]);
        let (_sink, ctx) = test_context();

        let default_id = actor.state.read().session.active_session_id().clone();

        // When handling EnvironmentLoaded.
        actor
            .on_environment_loaded(
                &crate::feat::provider_infra::ProvidersConfig {
                    providers: vec![],
                    aliases: vec![],
                    default_provider: None,
                },
                &ctx,
            )
            .await;

        // Then the default session still exists in the map.
        let state = actor.state.read();
        assert!(
            state.session.sessions().contains_key(&default_id),
            "default session should not be removed"
        );
    }

    #[tokio::test]
    async fn loading_unarchived_sessions_inserts_them_into_session_map() {
        // Given an actor with a default session and two sessions in the store.
        let store_session1 = ChatSessionState::new();
        let store_id1 = store_session1.session_id().clone();
        let store_session2 = ChatSessionState::new();
        let store_id2 = store_session2.session_id().clone();
        let actor = test_actor_with_store(vec![store_session1, store_session2]);
        let (_sink, ctx) = test_context();

        // When handling EnvironmentLoaded.
        actor
            .on_environment_loaded(
                &crate::feat::provider_infra::ProvidersConfig {
                    providers: vec![],
                    aliases: vec![],
                    default_provider: None,
                },
                &ctx,
            )
            .await;

        // Then both loaded sessions are in the session map.
        let state = actor.state.read();
        assert!(
            state.session.sessions().contains_key(&store_id1),
            "first store session should be in map"
        );
        assert!(
            state.session.sessions().contains_key(&store_id2),
            "second store session should be in map"
        );
    }
}
