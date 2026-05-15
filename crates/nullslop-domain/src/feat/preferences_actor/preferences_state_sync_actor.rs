//! Preferences state sync actor — keeps `AppState.frontend.preferences` in sync.
//!
//! Subscribes to [`PreferencesUpdated`] events emitted by [`PreferencesActor`].
//! On each event, replaces `state.frontend.preferences` with the full payload.
//! This is the ONLY actor that writes to `frontend.preferences`.

use crate::common::actor::{Actor, ActorContext, ActorEnvelope, NoDirectMsg};
use crate::common::state::State;
use crate::feat::theme;
use crate::protocol::Event;

/// Keeps `AppState.frontend.preferences` in sync with persisted preferences.
///
/// Subscribes to `PreferencesUpdated` events and writes the full preferences
/// to the shared state. This is the single writer for `frontend.preferences`.
pub struct PreferencesStateSyncActor {
    /// Shared application state.
    state: State,
}

impl Actor for PreferencesStateSyncActor {
    type Message = NoDirectMsg;

    #[expect(
        clippy::expect_used,
        reason = "State injection is required at activation"
    )]
    fn activate(ctx: &mut ActorContext) -> Self {
        ctx.subscribe_event::<super::protocol::event::PreferencesUpdated>();
        ctx.set_description("Syncs AppState.frontend.preferences from PreferencesUpdated events");

        let state = ctx
            .take_data::<State>()
            .expect("PreferencesStateSyncActor requires State injection");

        Self { state }
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, _ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Event(Event::PreferencesUpdated(ref payload)) => {
                let mut state = self.state.write();
                state.frontend.preferences = payload.preferences.clone();
                // Reload theme when theme_name changes in preferences.
                match theme::resolve_theme(payload.preferences.theme_name.as_deref()) {
                    Ok(t) => state.frontend.theme = t,
                    Err(e) => {
                        tracing::warn!(err = ?e, "failed to reload theme, keeping current");
                    }
                }
            }
            ActorEnvelope::Command(_) | ActorEnvelope::Event(_) | ActorEnvelope::System(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::common::actor::{
        Actor as _, ActorContext, ActorEnvelope, MessageSink, RecordingSink,
    };
    use crate::common::app_state::AppState;
    use crate::common::state::State;
    use crate::feat::preferences_actor::protocol::event::PreferencesUpdated;
    use crate::feat::preferences_actor::user_preferences::UserPreferences;
    use crate::protocol::Event;

    use super::PreferencesStateSyncActor;

    /// Creates a test actor with shared state.
    fn create_actor() -> (PreferencesStateSyncActor, State, ActorContext) {
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new("preferences-sync", sink.clone() as Arc<dyn MessageSink>);
        let state = State::new(AppState::default());
        ctx.set_data(state.clone());
        let actor = PreferencesStateSyncActor::activate(&mut ctx);
        (actor, state, ctx)
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn preferences_updated_syncs_to_app_state() {
        // Given a sync actor.
        let (mut actor, state, ctx) = create_actor();

        // When receiving PreferencesUpdated with a model and strategy.
        let prefs = UserPreferences {
            last_model: Some("ollama/llama3".to_owned()),
            last_strategy: Some("sliding_window".to_owned()),
            tool_result_max_lines: None,
            theme_name: None,
        };
        actor
            .handle(
                ActorEnvelope::Event(Event::PreferencesUpdated(PreferencesUpdated {
                    preferences: prefs,
                })),
                &ctx,
            )
            .await;

        // Then AppState.frontend.preferences is updated.
        let guard = state.read();
        assert_eq!(
            guard.frontend.preferences.last_model.as_deref(),
            Some("ollama/llama3")
        );
        assert_eq!(
            guard.frontend.preferences.last_strategy.as_deref(),
            Some("sliding_window")
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn second_event_overwrites_first() {
        // Given a sync actor with one update already applied.
        let (mut actor, state, ctx) = create_actor();
        let first = UserPreferences {
            last_model: Some("ollama/llama3".to_owned()),
            last_strategy: None,
            tool_result_max_lines: None,
            theme_name: None,
        };
        actor
            .handle(
                ActorEnvelope::Event(Event::PreferencesUpdated(PreferencesUpdated {
                    preferences: first,
                })),
                &ctx,
            )
            .await;

        // When receiving a second PreferencesUpdated.
        let second = UserPreferences {
            last_model: Some("openrouter/gpt-4".to_owned()),
            last_strategy: Some("sliding_window".to_owned()),
            tool_result_max_lines: None,
            theme_name: None,
        };
        actor
            .handle(
                ActorEnvelope::Event(Event::PreferencesUpdated(PreferencesUpdated {
                    preferences: second,
                })),
                &ctx,
            )
            .await;

        // Then AppState reflects the second update.
        let guard = state.read();
        assert_eq!(
            guard.frontend.preferences.last_model.as_deref(),
            Some("openrouter/gpt-4")
        );
        assert_eq!(
            guard.frontend.preferences.last_strategy.as_deref(),
            Some("sliding_window")
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn ignores_unrelated_events() {
        // Given a sync actor.
        let (mut actor, state, ctx) = create_actor();

        // When receiving an unrelated event (ModeChanged).
        actor
            .handle(
                ActorEnvelope::Event(Event::ModeChanged(crate::protocol::system::ModeChanged {
                    from: crate::protocol::Mode::Normal,
                    to: crate::protocol::Mode::Input,
                })),
                &ctx,
            )
            .await;

        // Then preferences remain at defaults.
        let guard = state.read();
        assert!(guard.frontend.preferences.last_model.is_none());
        assert!(guard.frontend.preferences.last_strategy.is_none());
    }
}
