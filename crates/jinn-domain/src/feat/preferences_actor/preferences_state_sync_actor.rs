//! Preferences state sync actor - keeps `AppState.frontend.preferences` in sync.
//!
//! Subscribes to [`PreferencesUpdated`] events emitted by [`PreferencesActor`].
//! On each event, replaces `state.frontend.preferences` with the full payload.
//! This is the ONLY actor that writes to `frontend.preferences`.

use super::protocol::event::PreferencesUpdated;
use crate::common::actor_deps::{ActorDeps, BusPublish};
use crate::common::services::bus_service::BusService;
use crate::common::state::State;
use kameo::prelude::{Actor, ActorRef, Context, Message};
/// Keeps `AppState.frontend.preferences` in sync with the persisted preferences.
///
/// Subscribes to `PreferencesUpdated` events and writes the full preferences
/// to the shared state. This is the single writer for `frontend.preferences`.
pub struct PreferencesStateSyncActor {
    /// Shared application state.
    state: State,
    deps: ActorDeps,
}

/// Dependencies for [`PreferencesStateSyncActor`].
#[derive(Clone)]
pub struct PreferencesStateSyncActorDeps {
    /// Shared application state.
    pub state: State,
    pub deps: ActorDeps,
}

impl Actor for PreferencesStateSyncActor {
    type Args = PreferencesStateSyncActorDeps;
    type Error = std::convert::Infallible;

    async fn on_start(args: Self::Args, actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        args.deps
            .subscribe(actor_ref.recipient::<PreferencesUpdated>())
            .await;

        Ok(Self {
            state: args.state,
            deps: args.deps,
        })
    }
}

impl BusPublish for PreferencesStateSyncActor {
    fn bus(&self) -> &BusService {
        &self.deps.services.bus
    }
}

impl Message<PreferencesUpdated> for PreferencesStateSyncActor {
    type Reply = ();

    async fn handle(&mut self, msg: PreferencesUpdated, _ctx: &mut Context<Self, Self::Reply>) {
        let mut state = self.state.write();
        state.frontend.preferences = msg.preferences;

        // If a project picker is open, its items are a snapshot taken at open
        // time. Reload them so adds/removes that round-trip through the
        // preferences actor are reflected immediately.
        if matches!(
            state.frontend.scope_stack.current(),
            crate::common::focus::FocusScope::Picker {
                kind: crate::feat::picker::PickerKind::Project
            }
        ) {
            crate::feat::picker::intent::load_project_picker_entries(&mut state);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::panic,
        clippy::unreachable,
        clippy::string_slice,
        clippy::uninlined_format_args,
        reason = "test code"
    )]
    use super::*;
    use crate::common::bus::test_harness::TestHarness;
    use crate::feat::preferences_actor::user_preferences::UserPreferences;

    async fn create_deps(harness: &TestHarness) -> PreferencesStateSyncActorDeps {
        PreferencesStateSyncActorDeps {
            state: State::new(crate::common::app_state::AppState::default()),
            deps: harness.actor_deps().await,
        }
    }

    #[tokio::test]
    async fn preferences_updated_syncs_to_frontend() {
        // Given a sync actor.
        let harness = TestHarness::new().await;
        let deps = create_deps(&harness).await;
        let state = deps.state.clone();
        let _actor = harness.spawn_actor::<PreferencesStateSyncActor>(deps).await;

        // When publishing a PreferencesUpdated event with custom preferences.
        let prefs = UserPreferences::default();
        harness
            .publish(PreferencesUpdated {
                preferences: prefs.clone(),
            })
            .await;

        // Then the frontend preferences match.
        // Give the bus time to deliver and the actor time to process.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let guard = state.read();
        assert_eq!(guard.frontend.preferences, prefs);
    }

    #[tokio::test]
    async fn preferences_updated_reloads_open_project_picker_items() {
        use crate::common::focus::FocusScope;
        use crate::feat::picker::PickerKind;
        use crate::feat::picker::intent::load_project_picker_entries;
        use crate::feat::project::ProjectConfig;
        use crate::feat::ui::picker_states::PickerExt;

        // Given a state with a project picker open and one stale entry.
        let harness = TestHarness::new().await;
        let deps = create_deps(&harness).await;
        let state = deps.state.clone();
        {
            let mut guard = state.write();
            // Seed the picker with one entry from default (empty) preferences.
            load_project_picker_entries(&mut guard);
            guard.frontend.scope_stack.push(FocusScope::Picker {
                kind: PickerKind::Project,
            });
            assert_eq!(
                guard.frontend.project_picker().items().len(),
                0,
                "picker starts empty with default preferences"
            );
        }
        let _actor = harness.spawn_actor::<PreferencesStateSyncActor>(deps).await;

        // When preferences update adds two projects.
        let prefs = UserPreferences {
            projects: vec![
                ProjectConfig {
                    path: std::path::PathBuf::from("/tmp/alpha"),
                },
                ProjectConfig {
                    path: std::path::PathBuf::from("/tmp/beta"),
                },
            ],
            ..UserPreferences::default()
        };
        harness
            .publish(PreferencesUpdated {
                preferences: prefs.clone(),
            })
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Then the open project picker's items are reloaded from the new prefs.
        let guard = state.read();
        assert_eq!(
            guard.frontend.project_picker().items().len(),
            2,
            "open project picker should reload items after preferences update"
        );
    }
}
