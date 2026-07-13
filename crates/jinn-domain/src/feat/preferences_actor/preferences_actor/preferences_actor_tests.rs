#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::indexing_slicing,
    reason = "test code"
)]

use std::time::Duration;

use super::*;
use crate::common::bus::test_harness::{TestHarness, await_recorded};
use crate::feat::preferences_actor::protocol::command::{PreferenceUpdate, UpdatePreferences};
use crate::feat::preferences_actor::protocol::event::PreferencesUpdated;

#[tokio::test]
async fn set_compaction_model_overwrites_previous() {
    // Given a preferences actor.
    let harness = TestHarness::new().await;
    let _actor = harness
        .spawn_actor::<PreferencesActor>(PreferencesActorDeps {
            deps: harness.actor_deps().await,
            state: crate::common::state::State::new(crate::common::app_state::AppState::default()),
        })
        .await;
    let recorder = harness.spawn_recorder::<PreferencesUpdated>().await;

    // When sending first UpdatePreferences.
    harness
        .publish(UpdatePreferences {
            updates: vec![PreferenceUpdate::SetCompactionModel(Some(
                "ollama/llama3".into(),
            ))],
        })
        .await;
    let messages = await_recorded(&recorder, 1, Duration::from_secs(2)).await;
    assert!(!messages.is_empty(), "expected first PreferencesUpdated");
    // When sending a second UpdatePreferences with a different model.
    harness
        .publish(UpdatePreferences {
            updates: vec![PreferenceUpdate::SetCompactionModel(Some(
                "openrouter/gpt-4".into(),
            ))],
        })
        .await;
    let messages = await_recorded(&recorder, 1, Duration::from_secs(2)).await;

    // Then only the latest model is in the emitted event.
    let found = messages
        .iter()
        .any(|e| e.preferences.compaction.model.as_deref() == Some("openrouter/gpt-4"));
    assert!(
        found,
        "expected PreferencesUpdated with compaction.model=openrouter/gpt-4, got {} events: {messages:?}",
        messages.len()
    );
}

#[tokio::test]
async fn emits_preferences_updated_event() {
    // Given a preferences actor and a recorder.
    let harness = TestHarness::new().await;
    let _actor = harness
        .spawn_actor::<PreferencesActor>(PreferencesActorDeps {
            deps: harness.actor_deps().await,
            state: crate::common::state::State::new(crate::common::app_state::AppState::default()),
        })
        .await;
    let recorder = harness.spawn_recorder::<PreferencesUpdated>().await;

    // When sending UpdatePreferences.
    harness
        .publish(UpdatePreferences {
            updates: vec![PreferenceUpdate::SetCompactionModel(Some(
                "ollama/llama3".into(),
            ))],
        })
        .await;

    // Then a PreferencesUpdated event was emitted with the full preferences.
    let messages = await_recorded(&recorder, 1, Duration::from_secs(2)).await;
    let found = messages
        .iter()
        .any(|e| e.preferences.compaction.model.as_deref() == Some("ollama/llama3"));
    assert!(
        found,
        "expected PreferencesUpdated event with compaction.model=ollama/llama3"
    );
}

#[tokio::test]
async fn empty_diffs_does_not_change_storage() {
    // Given a preferences actor.
    let harness = TestHarness::new().await;
    let _actor = harness
        .spawn_actor::<PreferencesActor>(PreferencesActorDeps {
            deps: harness.actor_deps().await,
            state: crate::common::state::State::new(crate::common::app_state::AppState::default()),
        })
        .await;
    let recorder = harness.spawn_recorder::<PreferencesUpdated>().await;

    // When setting a compaction model.
    harness
        .publish(UpdatePreferences {
            updates: vec![PreferenceUpdate::SetCompactionModel(Some(
                "ollama/llama3".into(),
            ))],
        })
        .await;
    let messages = await_recorded(&recorder, 1, Duration::from_secs(2)).await;
    assert!(!messages.is_empty(), "expected first PreferencesUpdated");

    // When sending UpdatePreferences with empty diffs.
    harness.publish(UpdatePreferences { updates: vec![] }).await;
    let messages = await_recorded(&recorder, 1, Duration::from_secs(2)).await;
    assert!(!messages.is_empty(), "expected second PreferencesUpdated");

    // Then the existing preferences are preserved.
    let found = messages
        .iter()
        .any(|e| e.preferences.compaction.model.as_deref() == Some("ollama/llama3"));
    assert!(found, "expected model to be preserved after empty update");
}
