//! Preferences actor - persists user preferences to `jinn.toml`.
//!
//! Subscribes to [`UpdatePreferences`] commands carrying batches of
//! [`PreferenceUpdate`] diffs. On each command, loads current preferences,
//! applies all diffs, saves to disk, and emits a [`PreferencesUpdated`]
//! event with the full result.
//!
//! # State ownership
//!
//! This actor does **not** write `AppState.frontend.preferences` directly.
//! That is the exclusive responsibility of `PreferencesStateSyncActor`,
//! which subscribes to the `PreferencesUpdated` events emitted here.

use crate::common::actor::{Actor, ActorContext, ActorEnvelope, NoDirectMsg};
use crate::common::services::Services;
use crate::feat::preferences_actor::protocol::command::UpdatePreferences;
use crate::feat::preferences_actor::protocol::event::PreferencesUpdated;

use crate::protocol::{Command, Event};

/// The preferences actor.
///
/// Subscribes to `UpdatePreferences` commands and persists preference
/// diffs to `jinn.toml`, then emits `PreferencesUpdated` so
/// downstream actors can sync their caches.
pub struct PreferencesActor {
    /// Runtime services.
    services: Services,
}
/// Dependencies for [`PreferencesActor`].
pub struct PreferencesActorDeps {
    /// Runtime services.
    pub services: Services,
}

impl Actor for PreferencesActor {
    type Message = NoDirectMsg;
    type Deps = PreferencesActorDeps;

    fn activate(deps: Self::Deps, ctx: &mut ActorContext) -> Self {
        ctx.subscribe_command::<UpdatePreferences>();
        ctx.set_description("Persists user preferences to jinn.toml");

        Self {
            services: deps.services,
        }
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Command(Command::UpdatePreferences(ref payload)) => {
                self.handle_update_preferences(payload, ctx);
            }
            ActorEnvelope::Event(_) | ActorEnvelope::Command(_) | ActorEnvelope::System(_) => {}
        }
    }
}

impl PreferencesActor {
    /// Processes a batch of preference diffs: load, apply, save, emit.
    fn handle_update_preferences(&self, payload: &UpdatePreferences, ctx: &ActorContext) {
        let mut prefs = self.services.user_preferences_storage.read();
        for update in &payload.updates {
            update.apply(&mut prefs);
        }
        if let Err(e) = self.services.user_preferences_storage.save(&prefs) {
            tracing::warn!(err = ?e, "preferences-actor failed to save user preferences");
            return;
        }
        if let Err(e) = ctx.send_event(Event::PreferencesUpdated(PreferencesUpdated {
            preferences: prefs,
        })) {
            tracing::warn!(err = ?e, "preferences-actor failed to emit PreferencesUpdated");
        }
    }
}

#[cfg(test)]
mod preferences_actor_tests;
