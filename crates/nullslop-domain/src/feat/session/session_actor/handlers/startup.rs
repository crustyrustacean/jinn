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

        let session_id;
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
            session_id = state.session.active_session_id().clone();
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

                    // Remove the empty default session if other sessions exist.
                    let default_is_empty = state.session.sessions().get(&session_id).is_some_and(
                        crate::feat::session::chat_session::ChatSessionState::is_empty,
                    );
                    if default_is_empty && state.session.sessions().len() > 1 {
                        state.session.sessions_mut().remove(&session_id);
                    }

                    // Set active to the most recently updated unarchived session.
                    if let Some(most_recent) = sorted.first() {
                        state.session.set_active(most_recent.session_id.clone());
                    }
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
