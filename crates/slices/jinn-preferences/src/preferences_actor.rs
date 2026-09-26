//! Preferences actor — persists user preferences to `jinn.toml`.
//!
//! A trouper [`ServiceActor`] declaring `UpdatePreferences` as handled;
//! handles [`UpdatePreferences`] commands carrying batches of
//! [`PreferenceUpdate`] diffs. On each command, loads current
//! preferences, applies all diffs, saves to disk, and writes
//! `frontend.preferences` inline after a successful save, reloading
//! the open project picker.

use jinn_domain::common::services::Services;
use jinn_domain::common::state::State;
use jinn_preferences_config::protocol::command::UpdatePreferences;
use jinn_project_msg::ProjectPickerState;
use jinn_slices::cell::TypedCell;
use trouper::actor::MsgHandler;
use trouper::actor::ServiceActor;
use trouper::builder::spawn_service_builder;
use trouper::context::MsgCtx;
use trouper::prelude::ActorPath;
use trouper::registry::RegistryError;
use trouper::system::ActorSystem;

/// The preferences actor's static trouper path.
pub const PREFERENCES_ACTOR_PATH: &str = "preferences-actor";

/// The preferences actor.
///
/// Subscribes to `UpdatePreferences` commands and persists preference
/// diffs to `jinn.toml`, writing `frontend.preferences` inline after a
/// successful save.
///
/// # State ownership
///
/// This actor owns `AppState.frontend.preferences` (authoritative writer).
/// It writes the field inline after persisting to `jinn.toml` — see the
/// "sync sibling" anti-pattern in AGENTS.md §3.
pub struct PreferencesActor {
    /// Runtime services (storage for load + save).
    services: Services,
    /// Shared application state — writes `frontend.preferences` inline after persist.
    state: State,
    /// The project picker's cell, so a persisted change to the curated project
    /// list refreshes an open menu. `None` when that slice is not activated.
    project_picker: Option<TypedCell<ProjectPickerState>>,
}

impl ServiceActor for PreferencesActor {
    #[expect(
        clippy::unused_async_trait_impl,
        reason = "trait contract: start is never called (spawn uses start_with)"
    )]
    async fn start(
        _args: &trouper::json::Json,
    ) -> Result<Self, error_stack::Report<RegistryError>> {
        // Never called: the spawn helper injects the state handle
        // via `start_with`.
        Err(
            error_stack::IntoReport::into_report(RegistryError::InvalidSpec)
                .attach("PreferencesActor is spawned via start_with"),
        )
    }
}

impl PreferencesActor {
    /// Spawns the actor at its static path, declaring `UpdatePreferences`
    /// as handled — the declaration registers the command's route (its
    /// sole handler), so bridge-published commands deliver here.
    pub fn spawn(
        system: &ActorSystem,
        services: Services,
        state: State,
        project_picker: Option<TypedCell<ProjectPickerState>>,
    ) -> ActorPath {
        spawn_service_builder::<Self>(system)
            .at(ActorPath::new(PREFERENCES_ACTOR_PATH))
            .start_with({
                move || {
                    Box::pin(async move {
                        Ok(Self {
                            services: services.clone(),
                            state: state.clone(),
                            project_picker: project_picker.clone(),
                        })
                    })
                }
            })
            .handles::<UpdatePreferences>()
            .start()
    }

    /// Processes a batch of preference diffs: load, apply, save, write inline.
    pub(crate) fn handle_update_preferences(&mut self, payload: &UpdatePreferences) {
        let mut prefs = self.services.user_preferences_storage.read();
        for update in &payload.updates {
            update.apply(&mut prefs);
        }
        if let Err(e) = self.services.user_preferences_storage.save(&prefs) {
            tracing::warn!(err = ?e, "preferences-actor failed to save user preferences");
            return;
        }

        // Write the persisted preferences into `frontend.preferences` inline, and
        // reload the open project picker so adds/removes round-tripping through
        // this actor are reflected immediately. The author of `frontend.preferences`
        // is this actor — keep the writes in one state guard.
        self.state.with_preferences(|view| {
            view.frontend().preferences = prefs.clone();
        });
        // The project picker is slice-owned and reads the curated list from
        // preferences, so a batch that adds or removes a project has to reach
        // its cell for the open menu to show the change. This slice names the
        // slot, not the picker: the kernel holds nothing for it either.
        if let Some(cell) = &self.project_picker {
            let projects = self.state.read().frontend.preferences.projects.clone();
            let theme = self.state.read().frontend.theme.clone();
            cell.update(|picker| {
                jinn_project::project_picker_actions::open(picker, &projects, &theme);
            });
        }
    }
}

impl MsgHandler<UpdatePreferences> for PreferencesActor {
    async fn handle(&mut self, msg: &UpdatePreferences, _ctx: &mut MsgCtx<'_>) {
        self.handle_update_preferences(msg);
    }
}

#[cfg(test)]
mod preferences_actor_tests;
