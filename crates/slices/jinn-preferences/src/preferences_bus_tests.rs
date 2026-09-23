// Copyright (C) 2026 Jayson Lennon
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as
// published by the Free Software Foundation, either version 3 of the
// License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! Bus-level integration tests for the preferences actors.
//!
//! Each test spawns the real actor via its trouper `spawn` (the exact
//! production wiring), publishes the command through the `BusService`
//! (the exact path the TUI bridge uses), and asserts the actor applied
//! the diff to storage and the inline frontend fields — proving the
//! command is routed to the actor, not silently dropped.

#![allow(clippy::expect_used, clippy::panic, reason = "test code")]

use std::time::Duration;

use jinn_domain::common::app_state::AppState;
use jinn_domain::common::bus::test_harness::TestHarness;
use jinn_domain::common::state::State;

/// Milliseconds per poll and total attempts for the delivery deadline.
const POLL_MS: u64 = 20;
const POLL_ATTEMPTS: usize = 100;

/// Spawns the real `PreferencesActor` over the harness fabric with a
/// harness-backed `Services` and a fresh shared `AppState`.
async fn preferences_setup() -> (TestHarness, jinn_domain::Services, State) {
    let harness = TestHarness::new().await;
    let services = harness.services().await;
    let state = State::new(AppState::default_with_scope_focus());
    let cap = jinn_domain::common::tcaps::mint::mint_frontend_cap();
    super::PreferencesActor::spawn(harness.system(), services.clone(), state.clone(), cap);
    (harness, services, state)
}

/// Spawns the real `AppStateActor` over the harness fabric with a
/// harness-backed `Services` and a fresh shared `AppState`.
async fn app_state_setup() -> (TestHarness, jinn_domain::Services, State) {
    let harness = TestHarness::new().await;
    let services = harness.services().await;
    let state = State::new(AppState::default_with_scope_focus());
    let cap = jinn_domain::common::tcaps::mint::mint_frontend_cap();
    super::AppStateActor::spawn(harness.system(), services.clone(), state.clone(), cap);
    (harness, services, state)
}

/// Polls until `f()` returns true or the poll budget is exhausted.
async fn poll_until<F, Fut>(mut f: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..POLL_ATTEMPTS {
        if f().await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(POLL_MS)).await;
    }
    f().await
}

#[rstest::rstest]
#[tokio::test]
async fn published_update_preferences_reaches_preferences_actor() {
    // Given a spawned preferences actor wired to the bus fabric.
    let (harness, services, state) = preferences_setup().await;

    // When publishing UpdatePreferences through the bus (the bridge path).
    harness
        .publish(jinn_preferences_config::protocol::command::UpdatePreferences {
            updates: vec![jinn_preferences_config::protocol::command::PreferenceUpdate::SetAccumulationThreshold(1000)],
        })
        .await;

    // Then the actor applied the diff to the persisted preferences.
    let applied = poll_until(|| async {
        services
            .user_preferences_storage
            .read()
            .auto_prune
            .accumulation_threshold_tokens
            == 1000
    })
    .await;
    assert!(
        applied,
        "UpdatePreferences must reach the actor and persist the threshold; persisted = {:?}",
        services.user_preferences_storage.read().auto_prune
    );
    // And the actor wrote frontend.preferences inline after persist.
    let inline = poll_until(|| async {
        state
            .read()
            .frontend
            .preferences
            .auto_prune
            .accumulation_threshold_tokens
            == 1000
    })
    .await;
    assert!(
        inline,
        "actor must write frontend.preferences inline; got {:?}",
        state.read().frontend.preferences.auto_prune
    );
}

#[rstest::rstest]
#[tokio::test]
async fn published_update_app_state_reaches_app_state_actor() {
    // Given a spawned app-state actor wired to the bus fabric.
    let (harness, services, state) = app_state_setup().await;

    // When publishing UpdateAppState through the bus (the bridge path).
    harness
        .publish(jinn_preferences_config::protocol::app_state_command::UpdateAppState {
            updates: vec![jinn_preferences_config::protocol::app_state_command::AppStateUpdate::SetSidebarWidth(Some(45))],
        })
        .await;

    // Then the actor applied the diff to the persisted app state.
    let applied =
        poll_until(|| async { services.app_state_storage.read().sidebar_width == Some(45) }).await;
    assert!(
        applied,
        "UpdateAppState must reach the actor and persist the sidebar width; persisted = {:?}",
        services.app_state_storage.read().sidebar_width
    );
    // And the actor synced the inline frontend field after persist.
    let inline = poll_until(|| async { state.read().frontend.sidebar_width == 45 }).await;
    assert!(
        inline,
        "actor must sync frontend.sidebar_width inline; got {}",
        state.read().frontend.sidebar_width
    );
}
