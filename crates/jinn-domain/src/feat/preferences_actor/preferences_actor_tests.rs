#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

use std::sync::Arc;

use crate::common::actor::{Actor as _, ActorContext, ActorEnvelope, MessageSink, RecordingSink};
use crate::common::services::Services;
use crate::feat::preferences_actor::preferences_actor::{PreferencesActor, PreferencesActorDeps};
use crate::feat::preferences_actor::protocol::command::{PreferenceUpdate, UpdatePreferences};
use crate::feat::preferences_actor::protocol::event::PreferencesUpdated;
use crate::protocol::{Command, Event};

/// Creates a test actor with in-memory storage.
fn create_actor() -> (PreferencesActor, Arc<RecordingSink>, ActorContext) {
    let sink = Arc::new(RecordingSink::new());
    let mut ctx = ActorContext::new("preferences-actor", sink.clone() as Arc<dyn MessageSink>);
    let services = Services::new();
    let actor = PreferencesActor::activate(PreferencesActorDeps { services }, &mut ctx);
    (actor, sink, ctx)
}

#[rstest::rstest]
#[tokio::test]
async fn set_compaction_model_saves_to_storage() {
    // Given a preferences actor with in-memory storage.
    let (mut actor, _sink, ctx) = create_actor();

    // When sending UpdatePreferences with SetCompactionModel.
    actor
        .handle(
            ActorEnvelope::Command(Command::UpdatePreferences(UpdatePreferences {
                updates: vec![PreferenceUpdate::SetCompactionModel(Some(
                    "ollama/llama3".into(),
                ))],
            })),
            &ctx,
        )
        .await;

    // Then the storage contains the model as compaction.model.
    let prefs = actor.services.user_preferences_storage.read();
    assert_eq!(
        prefs.compaction.model.as_deref(),
        Some("ollama/llama3")
    );
}

#[rstest::rstest]
#[tokio::test]
async fn set_compaction_model_overwrites_previous() {
    // Given a preferences actor with a saved compaction model.
    let (mut actor, _sink, ctx) = create_actor();
    actor
        .handle(
            ActorEnvelope::Command(Command::UpdatePreferences(UpdatePreferences {
                updates: vec![PreferenceUpdate::SetCompactionModel(Some(
                    "ollama/llama3".into(),
                ))],
            })),
            &ctx,
        )
        .await;

    // When sending a second UpdatePreferences with a different model.
    actor
        .handle(
            ActorEnvelope::Command(Command::UpdatePreferences(UpdatePreferences {
                updates: vec![PreferenceUpdate::SetCompactionModel(Some(
                    "openrouter/gpt-4".into(),
                ))],
            })),
            &ctx,
        )
        .await;

    // Then only the latest model is persisted.
    let prefs = actor.services.user_preferences_storage.read();
    assert_eq!(
        prefs.compaction.model.as_deref(),
        Some("openrouter/gpt-4")
    );
}

#[rstest::rstest]
#[tokio::test]
async fn emits_preferences_updated_event() {
    // Given a preferences actor.
    let (mut actor, sink, ctx) = create_actor();

    // When sending UpdatePreferences.
    actor
        .handle(
            ActorEnvelope::Command(Command::UpdatePreferences(UpdatePreferences {
                updates: vec![PreferenceUpdate::SetCompactionModel(Some(
                    "ollama/llama3".into(),
                ))],
            })),
            &ctx,
        )
        .await;

    // Then a PreferencesUpdated event was emitted with the full preferences.
    let events = sink.events();
    let found = events.iter().any(|e| {
        matches!(
            e,
            Event::PreferencesUpdated(PreferencesUpdated {
                preferences
            }) if preferences.compaction.model.as_deref() == Some("ollama/llama3")
        )
    });
    assert!(
        found,
        "expected PreferencesUpdated event with compaction.model=ollama/llama3"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn empty_diffs_does_not_change_storage() {
    // Given a preferences actor with a saved compaction model.
    let (mut actor, _sink, ctx) = create_actor();
    actor
        .handle(
            ActorEnvelope::Command(Command::UpdatePreferences(UpdatePreferences {
                updates: vec![PreferenceUpdate::SetCompactionModel(Some(
                    "ollama/llama3".into(),
                ))],
            })),
            &ctx,
        )
        .await;

    // When sending UpdatePreferences with empty diffs.
    actor
        .handle(
            ActorEnvelope::Command(Command::UpdatePreferences(UpdatePreferences {
                updates: vec![],
            })),
            &ctx,
        )
        .await;

    // Then the existing preferences are preserved.
    let prefs = actor.services.user_preferences_storage.read();
    assert_eq!(
        prefs.compaction.model.as_deref(),
        Some("ollama/llama3")
    );
}

#[rstest::rstest]
#[tokio::test]
async fn ignores_unrelated_commands() {
    // Given a preferences actor.
    let (mut actor, _sink, ctx) = create_actor();

    // When sending an unrelated command (RefreshModels).
    actor
        .handle(ActorEnvelope::Command(Command::RefreshModels), &ctx)
        .await;

    // Then no preferences were saved (still defaults).
    let prefs = actor.services.user_preferences_storage.read();
    assert!(prefs.compaction.model.is_none());
}
