//! Startup session hydration and persisted-default seeding.

use jinn_domain::common::actor_deps::BusPublish;
use jinn_preferences_config::protocol::app_state_command::{AppStateUpdate, UpdateAppState};
use jinn_session_msg::SessionSeed;
use jinn_session_store_msg::{SessionLoadCompleted, SessionSummary};

use crate::session_store_actor::SessionStoreActor;

impl SessionStoreActor {
    /// Applies persisted defaults and hydrates unarchived sessions.
    pub(crate) async fn on_environment_loaded(
        &self,
        _config: &jinn_provider_config::ProvidersConfig,
    ) {
        let app_state = self.services.app_state_storage.read();
        let welcome_mcp_enablement = self.seed_welcome_session(&app_state);
        self.state
            .with_frontend_app_state(|ops| ops.set(app_state.clone()));

        if let Some(enablement) = welcome_mcp_enablement {
            self.publish(enablement).await;
        }

        if !self.load_unarchived_sessions().await {
            return;
        }
        self.publish(UpdateAppState {
            updates: vec![AppStateUpdate::SetLastModel(app_state.last_model.clone())],
        })
        .await;
    }

    /// Applies persisted preferences to the welcome session only.
    fn seed_welcome_session(
        &self,
        app_state: &jinn_preferences_config::app_state_file::AppStateFile,
    ) -> Option<jinn_mcp_msg::McpEnablementChanged> {
        let mut enablement = None;
        self.state.with_session(|view| {
            let session = view.session.map().active_session_mut();
            if !session.profile().model.is_no_provider() {
                return;
            }
            if let Some(model) = &app_state.last_model {
                session.set_model(model.clone());
            }
            session.profile_mut().reasoning_effort = app_state.reasoning_effort;

            let seed = SessionSeed::from_config(&self.services.config);
            {
                let profile = session.profile_mut();
                profile.disabled_tools.clone_from(&seed.disabled_tools);
                profile.disabled_skills.clone_from(&seed.disabled_skills);
            }
            for server in &seed.enabled_mcp {
                session.enable_mcp_server(server);
            }
            if seed.has_auto_enabled_mcp() {
                enablement = Some(jinn_mcp_msg::McpEnablementChanged {
                    session_id: session.session_id().clone(),
                    enabled: seed.enabled_mcp,
                });
            }
        });
        enablement
    }

    /// Loads all unarchived sessions without switching the welcome session.
    ///
    /// Returns `false` when the store's summary read fails, preserving the
    /// existing startup path's early exit before `UpdateAppState`.
    async fn load_unarchived_sessions(&self) -> bool {
        self.state
            .with_session(|view| view.session.map().begin_startup_hydration());
        let summaries = match self
            .services
            .session_store
            .load_unarchived_summaries()
            .await
        {
            Ok(summaries) => summaries,
            Err(error) => {
                tracing::warn!(
                    ?error,
                    "session-actor failed to load unarchived summaries on startup"
                );
                self.finish_startup_hydration();
                return false;
            }
        };
        if summaries.is_empty() {
            self.finish_startup_hydration();
            return true;
        }

        let loaded_any = self.hydrate_unarchived_summaries(summaries).await;
        self.finish_startup_hydration();
        if !loaded_any {
            return true;
        }

        self.hydrate_all_tree_frozen_nodes(&self.services.session_store)
            .await;
        true
    }

    fn finish_startup_hydration(&self) {
        self.state
            .with_session(|view| view.session.map().finish_startup_hydration());
    }

    async fn hydrate_unarchived_summaries(&self, mut summaries: Vec<SessionSummary>) -> bool {
        summaries.sort_by_key(|summary| std::cmp::Reverse(summary.updated_at));
        let mut loaded_any = false;
        for summary in &summaries {
            let snapshot = match self
                .services
                .session_store
                .load_session(&summary.session_id)
                .await
            {
                Ok(Some(snapshot)) => snapshot,
                Ok(None) => {
                    tracing::warn!(
                        session_id = ?summary.session_id,
                        "session snapshot missing during startup hydration"
                    );
                    continue;
                }
                Err(error) => {
                    tracing::warn!(
                        ?error,
                        session_id = ?summary.session_id,
                        "failed to load session snapshot during startup hydration"
                    );
                    continue;
                }
            };
            loaded_any = true;
            let session_id = self.insert_loaded_session({
                let mut session = snapshot.restore_live();
                session.mark_interacted();
                session
            });
            self.publish(SessionLoadCompleted { session_id }).await;
        }
        loaded_any
    }
}
