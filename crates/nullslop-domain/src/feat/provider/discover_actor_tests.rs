//! Tests for DiscoverActor — command dispatch and handle routing.
//!
//! These tests kill mutants related to the actor's command dispatch
//! (handle, handle_command, RefreshModels match arm) by verifying
//! that commands are received and processed.
//!
//! Note: Testing the full `refresh_models` flow requires actual network calls,
//! so those specific mutants are killed by integration tests instead.

#![allow(clippy::expect_used, clippy::indexing_slicing)]

use std::sync::Arc;

use crate::common::actor::{
    Actor as _, ActorContext, ActorEnvelope, MessageSink, RecordingSink,
};
use crate::common::app_state::AppState;
use crate::common::services::Services;
use crate::common::state::State;
use crate::feat::provider::discover_actor::{DiscoverActor, DiscoverActorDeps};
use crate::protocol::Command;

fn create_actor() -> (DiscoverActor, Services, Arc<RecordingSink>, ActorContext, State) {
    let sink = Arc::new(RecordingSink::new());
    let mut ctx = ActorContext::new("discover", sink.clone() as Arc<dyn MessageSink>);

    let services = Services::new();
    let state = State::new(AppState::default());
    let deps = DiscoverActorDeps {
        registry: services.provider_registry.clone(),
        api_keys: services.api_keys.clone(),
        state: state.clone(),
        app_paths: services.paths.clone(),
    };
    let actor = DiscoverActor::activate(deps, &mut ctx);
    (actor, services, sink, ctx, state)
}

#[rstest::rstest]
#[tokio::test]
async fn handle_processes_command_envelope() {
    // Kills: replace handle with ().
    // Kills: replace handle_command with ().
    // If handle or handle_command were no-ops, the RefreshModels command
    // would be silently dropped and no event would be emitted.
    // We verify the command is processed by checking that the actor at least
    // attempts the discovery (emitting a ModelsRefreshed event with results,
    // even if the result maps are empty due to no configured providers).
    let (mut actor, _services, sink, ctx, _state) = create_actor();

    // When sending a RefreshModels command via ActorEnvelope::Command.
    actor
        .handle(ActorEnvelope::Command(Command::RefreshModels), &ctx)
        .await;

    // Then the actor processed the command — it should emit a ModelsRefreshed event
    // (with empty results since there are no configured providers).
    let events = sink.take_events();
    assert_eq!(events.len(), 1, "handle should dispatch RefreshModels and emit event");
}

#[rstest::rstest]
#[tokio::test]
async fn handle_ignores_event_envelope() {
    // Verifies that the actor properly distinguishes Command from Event envelopes.
    // This indirectly tests the match arm in handle().
    let (mut actor, _services, sink, ctx, _state) = create_actor();

    // When sending an unrelated event.
    actor
        .handle(
            ActorEnvelope::Event(crate::protocol::Event::ModeChanged(
                crate::protocol::system::ModeChanged {
                    from: crate::protocol::Mode::Normal,
                    to: crate::protocol::Mode::Input,
                },
            )),
            &ctx,
        )
        .await;

    // Then no events are emitted.
    let events = sink.take_events();
    assert!(events.is_empty(), "handle should ignore event envelopes");
}
