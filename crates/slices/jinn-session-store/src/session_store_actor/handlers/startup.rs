//! Startup session hydration and persisted-default seeding.

use jinn_domain::common::actor_deps::BusPublish;
use jinn_domain::feat::session::profile::SessionSeed;
use jinn_preferences_config::protocol::app_state_command::{AppStateUpdate, UpdateAppState};
use jinn_session_state::SessionSnapshot;
use jinn_session_store_msg::SessionLoadCompleted;

use crate::session_store_actor::SessionStoreActor;

impl SessionStoreActor {
    /// Applies persisted defaults and hydrates unarchived sessions.
    pub(crate) async fn on_environment_loaded(
        &self,
        _config: &jinn_domain::feat::provider_infra::ProvidersConfig,
    ) {
        let app_state = self.services.app_state_storage.read();
        let preferences = self.services.user_preferences_storage.read();
        let welcome_mcp_enablement = self.seed_welcome_session(&app_state, &preferences);
        self.state
            .with_frontend_app_state(&self.frontend_cap, |ops| ops.set(app_state.clone()));

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
        preferences: &jinn_preferences_config::UserPreferences,
    ) -> Option<jinn_mcp_msg::McpEnablementChanged> {
        let mut enablement = None;
        self.state.with_session(&self.session_cap, |view| {
            let session = view.session.map().active_session_mut();
            if !session.profile().model.is_no_provider() {
                return;
            }
            if let Some(model) = &app_state.last_model {
                session.set_model(model.clone());
            }
            session.profile_mut().reasoning_effort = app_state.reasoning_effort;

            let seed = SessionSeed::from_preferences(preferences);
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
                return false;
            }
        };
        if summaries.is_empty() {
            return true;
        }

        let loaded = self.load_summaries_in_recency_order(summaries).await;
        if loaded.is_empty() {
            return true;
        }
        for snapshot in loaded {
            let session_id = self.insert_loaded_session({
                let mut session = snapshot.restore_live();
                session.mark_interacted();
                session
            });
            self.publish(SessionLoadCompleted { session_id }).await;
        }

        self.hydrate_all_tree_frozen_nodes(&self.services.session_store)
            .await;
        true
    }

    /// Loads the complete sessions, newest summary first, outside the state lock.
    async fn load_summaries_in_recency_order(
        &self,
        mut summaries: Vec<jinn_domain::feat::session::SessionSummary>,
    ) -> Vec<SessionSnapshot> {
        summaries.sort_by_key(|summary| std::cmp::Reverse(summary.updated_at));
        let mut loaded = Vec::new();
        for summary in &summaries {
            if let Ok(Some(session)) = self
                .services
                .session_store
                .load_session(&summary.session_id)
                .await
            {
                loaded.push(session);
            }
        }
        loaded
    }
}
