#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::indexing_slicing,
    reason = "test code"
)]

use super::*;
use jinn_domain::common::app_state::AppState;
use jinn_domain::common::state::State;
use jinn_preferences_config::protocol::command::PreferenceUpdate;

async fn create_actor() -> (PreferencesActor, State) {
    let services = Services::new_fake().await;
    let state = State::new(AppState::default_with_scope_focus());
    let actor = PreferencesActor {
        services: services.clone(),
        state: state.clone(),
        project_picker: None,
    };
    (actor, state)
}

#[rstest::rstest]
#[tokio::test]
async fn set_compaction_model_overwrites_previous() {
    // Given a preferences actor.
    let (mut actor, _state) = create_actor().await;

    // When applying the first update.
    actor.handle_update_preferences(&UpdatePreferences {
        updates: vec![PreferenceUpdate::SetCompactionModel(Some(
            "ollama/llama3".into(),
        ))],
    });
    // When applying a second update with a different model.
    actor.handle_update_preferences(&UpdatePreferences {
        updates: vec![PreferenceUpdate::SetCompactionModel(Some(
            "openrouter/gpt-4".into(),
        ))],
    });

    // Then only the latest model is persisted.
    let prefs = actor.services.user_preferences_storage.read();
    assert_eq!(
        prefs.compaction.model.as_deref(),
        Some("openrouter/gpt-4"),
        "expected persisted compaction.model=openrouter/gpt-4"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn update_persists_applied_preferences() {
    // Given a preferences actor.
    let (mut actor, _state) = create_actor().await;

    // When applying an update.
    actor.handle_update_preferences(&UpdatePreferences {
        updates: vec![PreferenceUpdate::SetCompactionModel(Some(
            "ollama/llama3".into(),
        ))],
    });

    // Then the full preferences are persisted with the applied model.
    let prefs = actor.services.user_preferences_storage.read();
    assert_eq!(
        prefs.compaction.model.as_deref(),
        Some("ollama/llama3"),
        "expected persisted compaction.model=ollama/llama3"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn empty_diffs_does_not_change_storage() {
    // Given a preferences actor with a model already set.
    let (mut actor, _state) = create_actor().await;
    actor.handle_update_preferences(&UpdatePreferences {
        updates: vec![PreferenceUpdate::SetCompactionModel(Some(
            "ollama/llama3".into(),
        ))],
    });

    // When applying an update with empty diffs.
    actor.handle_update_preferences(&UpdatePreferences { updates: vec![] });

    // Then the existing preferences are preserved.
    let prefs = actor.services.user_preferences_storage.read();
    assert_eq!(
        prefs.compaction.model.as_deref(),
        Some("ollama/llama3"),
        "expected model to be preserved after empty update"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn persist_writes_frontend_preferences() {
    // Given a preferences actor.
    let (mut actor, state) = create_actor().await;

    // When applying an update.
    actor.handle_update_preferences(&UpdatePreferences {
        updates: vec![PreferenceUpdate::SetCompactionModel(Some(
            "ollama/llama3".into(),
        ))],
    });

    // Then frontend.preferences matches the persisted preferences.
    let guard = state.read();
    assert_eq!(
        guard.frontend.preferences.compaction.model.as_deref(),
        Some("ollama/llama3"),
        "frontend.preferences must be written inline after persist"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn persist_refreshes_the_open_project_picker() {
    use jinn_project::project_picker_actions;
    use jinn_project_msg::{ProjectPickerState, project_picker_slot};
    use jinn_slices::Slices;
    use jinn_slices::cell::TypedCell;

    // Given a preferences actor holding the project picker's cell, and a
    // picker opened on the default (empty) curated list.
    let slices = Slices::new();
    let cell: TypedCell<ProjectPickerState> = slices
        .register(project_picker_slot(), ProjectPickerState::default())
        .expect("the picker cell registers once");
    let (mut actor, state) = create_actor().await;
    actor.project_picker = Some(cell.clone());
    cell.update(|picker| {
        project_picker_actions::open(
            picker,
            &state.read().frontend.preferences.projects,
            &state.read().frontend.theme,
        );
    });
    assert_eq!(
        cell.read().selection.items().len(),
        0,
        "picker starts empty"
    );

    // When preferences update adds two projects.
    actor.handle_update_preferences(&UpdatePreferences {
        updates: vec![
            PreferenceUpdate::AddProject(std::path::PathBuf::from("/tmp/alpha")),
            PreferenceUpdate::AddProject(std::path::PathBuf::from("/tmp/beta")),
        ],
    });

    // Then the open picker shows both, without the kernel knowing it exists.
    assert_eq!(
        cell.read().selection.items().len(),
        2,
        "the open project picker should refresh after a preferences update"
    );
}
