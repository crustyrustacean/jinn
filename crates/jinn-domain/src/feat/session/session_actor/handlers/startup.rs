//! Startup handler - applies config defaults on environment load.
//!
//! On startup, loads user preferences, applies model/strategy defaults to the
//! default session, loads unarchived sessions from SQLite, and emits commands
//! to initialize the context and preferences pipelines.

use super::super::SessionPersistenceActor;
use crate::common::actor_deps::BusPublish;
use crate::feat::preferences_actor::protocol::app_state_command::{AppStateUpdate, UpdateAppState};

impl SessionPersistenceActor {
    /// Applies config defaults to the default session profile on startup.
    ///
    /// Loads app state and applies `last_model`
    /// to the default session, then sends an `UpdateAppState` command so
    /// the state pipeline handles persistence and state sync.
    ///
    /// NOTE: Using `active_session_mut()` is acceptable here because this runs
    /// at startup before any user interaction. There is only one session.
    pub(in crate::feat::session::session_actor) async fn on_environment_loaded(
        &self,
        _config: &crate::feat::provider_infra::ProvidersConfig,
    ) {
        let app_state = self.services.app_state_storage.read();

        {
            let mut state = self.state.write();

            // Apply config defaults to the default session.
            // Only set the model if the session still has the no-provider sentinel.
            // Bench sessions are created with an explicit model before this handler
            // fires, so we must not overwrite them with the user's saved preference.
            let session = state.active_session_mut();
            if let Some(ref model) = app_state.last_model
                && session.profile().model.is_no_provider()
            {
                session.set_model(model.clone());
            }
            // Seed frontend.app_state from persisted state so the persona scan
            // (which fires on the same EnvironmentLoaded event) finds persona_name
            // populated when on_personas_loaded resolves the active persona.
            state.frontend.app_state = app_state.clone();
        }

        tracing::info!("DIAG on_environment_loaded model/strategy applied");

        // Note: no scan commands are emitted here. The three scan actors
        // (skills, prompts, context-files) subscribe to this `EnvironmentLoaded`
        // event directly and self-trigger their per-session scans.

        tracing::info!("DIAG on_environment_loaded loading unarchived sessions");

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
            tracing::info!(
                count = summaries.len(),
                "DIAG on_environment_loaded summaries loaded"
            );

            if !summaries.is_empty() {
                // Sort by updated_at descending to find the most recent.
                let mut sorted = summaries;
                sorted.sort_by_key(|s| std::cmp::Reverse(s.updated_at));

                // Load full sessions outside the lock.
                let mut loaded = Vec::new();
                for summary in &sorted {
                    if let Ok(Some(session)) = store.load_session(&summary.session_id).await {
                        loaded.push(session);
                    }
                }
                tracing::info!(
                    count = loaded.len(),
                    "DIAG on_environment_loaded sessions loaded"
                );

                tracing::info!("DIAG on_environment_loaded inserting sessions");
                if !loaded.is_empty() {
                    for mut session in loaded {
                        // Mark startup-loaded sessions as interacted - they came from disk.
                        session.mark_interacted();
                        let session_id = session.session_id().clone();
                        self.load_and_insert(session).await;
                        self.rehydrate_attached_plugins(&session_id);
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
                    tracing::info!("DIAG on_environment_loaded frozen nodes hydrated");
                }
            }
        }

        tracing::info!("DIAG on_environment_loaded sending UpdateAppState");
        // Send UpdateAppState command so the pipeline handles persistence + state sync.
        self.publish(UpdateAppState {
            updates: vec![AppStateUpdate::SetLastModel(app_state.last_model.clone())],
        })
        .await;
        tracing::info!("DIAG on_environment_loaded DONE");
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
    use super::super::super::helpers::test_actor_with_store_recording;
    use crate::feat::session::chat_session::ChatSessionState;
    use crate::feat::session::model_selection::ModelSelection;

    #[tokio::test]
    async fn loading_unarchived_sessions_does_not_switch_active_session() {
        // Given an actor with a default welcome session and one session in the store.
        let store_session = ChatSessionState::new();
        let (actor, _store, _audit) = test_actor_with_store_recording(vec![store_session]).await;

        // Record the default session's ID before loading.
        let default_id = actor.state.read().session.active_session_id().clone();

        // When handling EnvironmentLoaded.
        actor
            .on_environment_loaded(&crate::feat::provider_infra::ProvidersConfig {
                providers: vec![],
                aliases: vec![],
                default_provider: None,
            })
            .await;

        // Then the active session is still the default.
        let state = actor.state.read();
        assert_eq!(*state.session.active_session_id(), default_id);
    }

    #[tokio::test]
    async fn on_environment_loaded_emits_no_scan_commands() {
        // Given an actor with a default welcome session.
        let (actor, _store, audit) = test_actor_with_store_recording(vec![]).await;

        // When handling EnvironmentLoaded.
        actor
            .on_environment_loaded(&crate::feat::provider_infra::ProvidersConfig {
                providers: vec![],
                aliases: vec![],
                default_provider: None,
            })
            .await;

        // Then no scan commands are emitted. The three scan actors
        // (skills, prompts, context-files) subscribe to this `EnvironmentLoaded`
        // event directly and self-trigger their per-session scans.
        assert!(
            !audit.contains_name("ScanSkills"),
            "should not emit ScanSkills"
        );
        assert!(
            !audit.contains_name("RescanPromptTemplates"),
            "should not emit RescanPromptTemplates"
        );
        assert!(
            !audit.contains_name("ScanContextFiles"),
            "should not emit ScanContextFiles"
        );
    }

    #[tokio::test]
    async fn loading_unarchived_sessions_does_not_remove_default_session() {
        // Given an actor with a default welcome session and one session in the store.
        let store_session = ChatSessionState::new();
        let (actor, _store, _audit) = test_actor_with_store_recording(vec![store_session]).await;

        let default_id = actor.state.read().session.active_session_id().clone();

        // When handling EnvironmentLoaded.
        actor
            .on_environment_loaded(&crate::feat::provider_infra::ProvidersConfig {
                providers: vec![],
                aliases: vec![],
                default_provider: None,
            })
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
        let (actor, _audit, _store) =
            test_actor_with_store_recording(vec![store_session1, store_session2]).await;

        // When handling EnvironmentLoaded.
        actor
            .on_environment_loaded(&crate::feat::provider_infra::ProvidersConfig {
                providers: vec![],
                aliases: vec![],
                default_provider: None,
            })
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
        let (actor, _store, _audit) = test_actor_with_store_recording(vec![]).await;

        // Save state with a last_model.
        let state_file = crate::feat::preferences_actor::app_state_file::AppStateFile {
            last_model: Some(ModelSelection::from_single("my-model".to_owned())),
            ..Default::default()
        };
        actor
            .services
            .app_state_storage
            .save(&state_file)
            .expect("save state");

        // When handling EnvironmentLoaded.
        actor
            .on_environment_loaded(&crate::feat::provider_infra::ProvidersConfig {
                providers: vec![],
                aliases: vec![],
                default_provider: None,
            })
            .await;

        // Then the active session's model was updated from the saved preference.
        let state = actor.state.read();
        assert_eq!(
            state.active_session().profile().model,
            ModelSelection::Single("my-model".to_owned()),
            "default session model should be updated from saved preference"
        );
    }

    #[tokio::test]
    async fn saved_model_does_not_overwrite_explicitly_set_model() {
        // Given an actor with a session that has an explicit model (not NO_PROVIDER_ID)
        // and saved preferences with a different last_model.
        let (actor, _store, _audit) = test_actor_with_store_recording(vec![]).await;

        // Set an explicit model on the active session (simulating bench actor behavior).
        {
            let mut state = actor.state.write();
            state
                .active_session_mut()
                .set_model(ModelSelection::Single("bench-model".to_owned()));
        }

        // Save state with a different last_model.
        let state_file = crate::feat::preferences_actor::app_state_file::AppStateFile {
            last_model: Some(ModelSelection::from_single("wrong-model".to_owned())),
            ..Default::default()
        };
        actor
            .services
            .app_state_storage
            .save(&state_file)
            .expect("save state");

        // When handling EnvironmentLoaded.
        actor
            .on_environment_loaded(&crate::feat::provider_infra::ProvidersConfig {
                providers: vec![],
                aliases: vec![],
                default_provider: None,
            })
            .await;

        let state = actor.state.read();
        assert_eq!(
            state.active_session().profile().model,
            ModelSelection::Single("bench-model".to_owned()),
            "explicitly set model should not be overwritten by saved preference"
        );
    }

    #[tokio::test]
    async fn startup_rehydrates_attached_plugins_for_loaded_sessions() {
        // Given a session in the store with an attached plugin.
        let mut store_session = ChatSessionState::new();
        let ap = jinn_core_types::AttachedPlugin::new("test");
        store_session.core.attached_plugins.push(ap);
        let (actor, _audit, _store) = test_actor_with_store_recording(vec![store_session]).await;

        // When handling EnvironmentLoaded (startup).
        actor
            .on_environment_loaded(&crate::feat::provider_infra::ProvidersConfig {
                providers: vec![],
                aliases: vec![],
                default_provider: None,
            })
            .await;

        // Then the attached plugin was loaded with the session.
        let state = actor.state.read();
        let session = state
            .session
            .iter()
            .find(|(_, s)| !s.core.attached_plugins.is_empty());
        assert!(
            session.is_some(),
            "session with attached plugin should be loaded after startup"
        );
    }

    #[tokio::test]
    async fn startup_resets_running_plugins_to_idle() {
        let mut store_session = ChatSessionState::new();
        let ap = jinn_core_types::AttachedPlugin::new("test");
        // Force into Running state to simulate crash.
        store_session
            .core
            .attached_plugins
            .push(jinn_core_types::AttachedPlugin {
                run_state: jinn_core_types::PluginRunState::Running,
                ..ap
            });
        let session_id = store_session.session_id().clone();
        let (actor, _audit, _store) = test_actor_with_store_recording(vec![store_session]).await;

        // When handling EnvironmentLoaded (startup).
        actor
            .on_environment_loaded(&crate::feat::provider_infra::ProvidersConfig {
                providers: vec![],
                aliases: vec![],
                default_provider: None,
            })
            .await;

        // Then the plugin was reset from Running to Idle.
        let state = actor.state.read();
        let session = state
            .session
            .get(&session_id)
            .expect("session should be loaded");
        let ap = &session.core.attached_plugins[0];
        assert!(
            matches!(ap.run_state, jinn_core_types::PluginRunState::Idle),
            "Running plugin should be reset to Idle on startup, got {:?}",
            ap.run_state
        );
    }

    #[tokio::test]
    async fn startup_seeds_persisted_persona_into_frontend_app_state() {
        // Given an actor and saved app state with a persona_name.
        let (actor, _store, _audit) = test_actor_with_store_recording(vec![]).await;
        let state_file = crate::feat::preferences_actor::app_state_file::AppStateFile {
            persona_name: Some("general".to_owned()),
            ..Default::default()
        };
        actor
            .services
            .app_state_storage
            .save(&state_file)
            .expect("save state");

        // When handling EnvironmentLoaded.
        actor
            .on_environment_loaded(&crate::feat::provider_infra::ProvidersConfig {
                providers: vec![],
                aliases: vec![],
                default_provider: None,
            })
            .await;

        // Then frontend.app_state.persona_name is seeded so the persona scan
        // (which fires on the same EnvironmentLoaded event) can resolve it.
        let state = actor.state.read();
        assert_eq!(
            state.frontend.app_state.persona_name.as_deref(),
            Some("general"),
            "startup should seed frontend.app_state from persisted state"
        );
    }

    #[tokio::test]
    async fn startup_seeds_none_persona_when_state_absent() {
        // Given an actor with default (empty) app state storage.
        let (actor, _store, _audit) = test_actor_with_store_recording(vec![]).await;

        // When handling EnvironmentLoaded.
        actor
            .on_environment_loaded(&crate::feat::provider_infra::ProvidersConfig {
                providers: vec![],
                aliases: vec![],
                default_provider: None,
            })
            .await;

        // Then frontend.app_state.persona_name is None (falls back to
        // DEFAULT_PERSONA_NAME downstream in on_personas_loaded, not here).
        let state = actor.state.read();
        assert_eq!(
            state.frontend.app_state.persona_name, None,
            "startup should not fabricate a persona name when none is persisted"
        );
    }
}
