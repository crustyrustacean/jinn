//! Startup handler - applies config defaults on environment load.
//!
//! On startup, loads user preferences, applies model/strategy defaults to the
//! default session, loads unarchived sessions from SQLite, and emits commands
//! to initialize the context and preferences pipelines.

use crate::common::actor::ActorContext;
use crate::feat::context::env_context::load_project_context_files;
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
        let prefs = match self.services.user_preferences_storage.load() {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(err = ?e, "session-actor failed to load preferences on startup");
                return;
            }
        };

        {
            let mut state = self.state.write();

            // Apply config defaults to the default session.
            // Only set the model if the session still has the no-provider sentinel.
            // Bench sessions are created with an explicit model before this handler
            // fires, so we must not overwrite them with the user's saved preference.
            let session = state.active_session_mut();
            if let Some(ref model) = prefs.last_model
                && session.profile().model == crate::feat::provider_infra::NO_PROVIDER_ID
            {
                session.set_model(model.clone());
            }
            if let Some(ref strategy_str) = prefs.last_strategy {
                let strategy_id = PromptStrategyId::new(strategy_str.clone());
                session.switch_strategy(strategy_id.clone());
            }
        }

        // Load project context files (AGENTS.md/CLAUDE.md) from CWD tree into cache.
        // This replaces per-assembly disk reads with a one-time startup load.
        let cwd = {
            let state = self.state.read();
            state.active_session().cwd().to_path_buf()
        };
        let context_files = load_project_context_files(&cwd).await;
        if !context_files.is_empty() {
            let mut state = self.state.write();
            state.context.context_files = context_files;
        }

        // Load unarchived sessions from SQLite into memory.
        // These are sessions with `archived=false` - corresponding to `SessionState::Loaded`.
        // On load they get the default `SessionState::Loaded` and `LifecycleScriptState::NothingRan`.
        {
            let store = &self.services.session_store;
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
                    for mut session in loaded {
                        // Mark startup-loaded sessions as interacted - they came from disk.
                        session.mark_interacted();
                        let session_id = session.session_id().clone();
                        self.load_and_insert(session, ctx);
                        self.rehydrate_attached_workflows(&session_id);
                    }

                    // NOTE: We intentionally do NOT switch the active session.
                    // The user should land on the fresh welcome session.
                    // Previously loaded sessions appear in the sidebar but
                    // don't steal focus.

                    // Hydrate frozen nodes for archived tree members.
                    // Archived sessions (e.g., a parent that was archived while its
                    // children remain unarchived) need frozen node snapshots so the
                    // tree summary shows complete historical totals.
                    self.hydrate_all_tree_frozen_nodes(&self.services.session_store)
                        .await;
                }
            }
        }

        // Send UpdatePreferences command so the pipeline handles persistence + state sync.
        if let Err(e) = ctx.send_command(Command::UpdatePreferences(crate::feat::preferences_actor::protocol::command::UpdatePreferences {
                updates: vec![
                    crate::feat::preferences_actor::protocol::command::PreferenceUpdate::SetLastModel(prefs.last_model.clone()),
                ],
            })) {
            tracing::warn!(err = ?e, "session-actor failed to send UpdatePreferences on startup");
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use super::super::super::helpers::{test_actor_with_store, test_context};
    use crate::feat::session::chat_session::ChatSessionState;

    #[tokio::test]
    async fn loading_unarchived_sessions_does_not_switch_active_session() {
        // Given an actor with a default welcome session and one session in the store.
        let store_session = ChatSessionState::new();
        let (actor, _store) = test_actor_with_store(vec![store_session]);
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
        let (actor, _store) = test_actor_with_store(vec![store_session]);
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
            state.session.contains(&default_id),
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
        let (actor, _store) = test_actor_with_store(vec![store_session1, store_session2]);
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
            state.session.contains(&store_id1),
            "first store session should be in map"
        );
        assert!(
            state.session.contains(&store_id2),
            "second store session should be in map"
        );
    }

    #[tokio::test]
    async fn saved_model_overwrites_no_provider_sentinel() {
        // Given an actor with a default session (NO_PROVIDER_ID model)
        // and saved preferences with a last_model.
        let (actor, _store) = test_actor_with_store(vec![]);
        let (_sink, ctx) = test_context();

        // Save preferences with a last_model.
        let prefs = crate::feat::preferences_actor::user_preferences::UserPreferences {
            last_model: Some("my-model".to_owned()),
            ..Default::default()
        };
        actor
            .services
            .user_preferences_storage
            .save(&prefs)
            .expect("save prefs");

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

        // Then the active session's model was updated from the saved preference.
        let state = actor.state.read();
        assert_eq!(
            state.active_session().profile().model,
            "my-model",
            "default session model should be updated from saved preference"
        );
    }

    #[tokio::test]
    async fn saved_model_does_not_overwrite_explicitly_set_model() {
        // Given an actor with a session that has an explicit model (not NO_PROVIDER_ID)
        // and saved preferences with a different last_model.
        let (actor, _store) = test_actor_with_store(vec![]);

        // Set an explicit model on the active session (simulating bench actor behavior).
        {
            let mut state = actor.state.write();
            state
                .active_session_mut()
                .set_model("bench-model".to_owned());
        }

        // Save preferences with a different last_model.
        let prefs = crate::feat::preferences_actor::user_preferences::UserPreferences {
            last_model: Some("wrong-model".to_owned()),
            ..Default::default()
        };
        actor
            .services
            .user_preferences_storage
            .save(&prefs)
            .expect("save prefs");

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

        // Then the active session's model was NOT overwritten.
        let state = actor.state.read();
        assert_eq!(
            state.active_session().profile().model,
            "bench-model",
            "explicitly set model should not be overwritten by saved preference"
        );
    }

    #[tokio::test]
    async fn startup_rehydrates_attached_workflows_for_loaded_sessions() {
        // Given a session in the store with an attached workflow.
        let mut store_session = ChatSessionState::new();
        let wf_id = crate::feat::workflow::workflow_state::WorkflowId::new();
        store_session.core.attached_workflows.push(
            crate::feat::workflow::attached_workflow::AttachedWorkflow {
                id: wf_id.clone(),
                config: crate::feat::workflow::attached_workflow::WorkflowConfig::Consensus {
                    n: 3,
                    result_kind: crate::feat::workflow::attached_workflow::ResultKind::Assistant,
                },
                label: "Consensus".to_owned(),
                trigger: crate::feat::workflow::attached_workflow::WorkflowTrigger::Manual,
                enabled: true,
                state: crate::feat::workflow::attached_workflow::AttachedWorkflowState::Ready,
            },
        );
        let (actor, _store) = test_actor_with_store(vec![store_session]);
        let (_sink, ctx) = test_context();

        // When handling EnvironmentLoaded (startup).
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

        // Then the workflow is registered in the runtime workflow map.
        let state = actor.state.read();
        assert!(
            state.workflow.get(&wf_id).is_some(),
            "attached workflow should be re-registered in state.workflow after startup"
        );
    }

    #[tokio::test]
    async fn startup_resets_running_workflows_to_ready() {
        // Given a session in the store with a workflow stuck in Running state (crash simulation).
        let mut store_session = ChatSessionState::new();
        let wf_id = crate::feat::workflow::workflow_state::WorkflowId::new();
        store_session.core.attached_workflows.push(
            crate::feat::workflow::attached_workflow::AttachedWorkflow {
                id: wf_id.clone(),
                config: crate::feat::workflow::attached_workflow::WorkflowConfig::Consensus {
                    n: 3,
                    result_kind: crate::feat::workflow::attached_workflow::ResultKind::Assistant,
                },
                label: "Consensus".to_owned(),
                trigger: crate::feat::workflow::attached_workflow::WorkflowTrigger::Manual,
                enabled: true,
                state: crate::feat::workflow::attached_workflow::AttachedWorkflowState::Running,
            },
        );
        let session_id = store_session.session_id().clone();
        let (actor, _store) = test_actor_with_store(vec![store_session]);
        let (_sink, ctx) = test_context();

        // When handling EnvironmentLoaded (startup).
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

        // Then the workflow was reset from Running to Ready.
        let state = actor.state.read();
        let session = state
            .session
            .get(&session_id)
            .expect("session should be loaded");
        let aw = &session.core.attached_workflows[0];
        assert!(
            matches!(
                aw.state,
                crate::feat::workflow::attached_workflow::AttachedWorkflowState::Ready
            ),
            "Running workflow should be reset to Ready on startup, got {:?}",
            aw.state
        );
    }
}
