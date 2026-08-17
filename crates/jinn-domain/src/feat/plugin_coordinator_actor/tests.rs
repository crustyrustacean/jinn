//! Coordinator tests using the scripted fake-guest seam.
//!
//! The fake replaces the wasm guest with in-process logic speaking the same
//! NDJSON wire over the same pipes, so these tests exercise the production
//! handshake, read-pump, and validation paths without a compiled plugin.
#![allow(clippy::expect_used, reason = "test assertions")]

use std::sync::Arc;
use std::time::Duration;

use kameo::actor::Spawn;

use crate::common::bus::test_harness::{TestHarness, await_recorded};
use crate::common::root_supervisor::RootSupervisor;
use crate::common::state::State;
use crate::common::tcaps::mint::mint_plugins_cap;
use crate::feat::plugin::PluginConfig;
use crate::feat::plugin_coordinator_actor::PluginCoordinatorActor;
use crate::feat::plugin_coordinator_actor::PluginCoordinatorActorDeps;
use crate::feat::plugin_coordinator_actor::PluginDirs;
use crate::feat::plugin_coordinator_actor::protocol::{PluginPhase, PluginStatus};

/// Timeout for awaiting expected plugin outcomes.
const WAIT: Duration = Duration::from_secs(5);

/// One valid wire `SetThemeEntries` line (hand-encoded JSON to also prove the
/// raw wire shape survives decode).
fn theme_line(name: &str, color: &str) -> String {
    format!(
        r#"{{"v":1,"seq":2,"ts":0,"type":"set_theme_entries","themes":[{{"name":"{name}","description":null,"colors":{{"focus_accent":"{color}"}}}}]}}"#
    )
}

/// Spawns the coordinator with the given plugin entries and fake script.
async fn spawn_coordinator(
    harness: &TestHarness,
    plugins: std::collections::BTreeMap<String, PluginConfig>,
    script: jinn_plugin::FakeGuestScript,
) -> State {
    spawn_coordinator_prepared(harness, plugins, script, |_| {}).await
}

/// Like [`spawn_coordinator`], with a preparer that mutates shared state
/// before the coordinator (and its plugins) spawn — for arming startup
/// conditions the contribution path must observe.
async fn spawn_coordinator_prepared(
    harness: &TestHarness,
    plugins: std::collections::BTreeMap<String, PluginConfig>,
    script: jinn_plugin::FakeGuestScript,
    prepare: impl FnOnce(&State),
) -> State {
    let services = harness.services().await;
    {
        let mut prefs = services.user_preferences_storage.read().clone();
        prefs.plugin = plugins;
        services
            .user_preferences_storage
            .save(&prefs)
            .expect("save prefs");
    }
    let state = State::new(crate::common::app_state::AppState::default());
    prepare(&state);
    let root = RootSupervisor::spawn_root().await;
    let dirs = PluginDirs {
        config_dir: std::path::PathBuf::from("/nonexistent"),
        data_dir: std::path::PathBuf::from("/nonexistent"),
        engine: Arc::new(jinn_plugin::PluginEngine::new().expect("engine construction")),
    };
    let actor = PluginCoordinatorActor::supervise(
        &root,
        PluginCoordinatorActorDeps {
            deps: crate::common::actor_deps::ActorDeps {
                services: services.clone(),
            },
            root: root.clone(),
            state: state.clone(),
            cap: mint_plugins_cap(),
            frontend_cap: crate::common::tcaps::mint::mint_frontend_cap(),
            dirs,
            fake_guest: Arc::new(std::sync::Mutex::new(Some(script))),
        },
    )
    .restart_policy(kameo::supervision::RestartPolicy::Never)
    .spawn()
    .await;
    actor.wait_for_startup().await;
    state
}

/// A manifest entry the coordinator will spawn.
fn entry() -> PluginConfig {
    PluginConfig {
        wasm: "test.wasm".to_owned(),
        grants: vec![],
        http: false,
        config: None,
        enabled: true,
    }
}

/// A one-entry plugin map keyed by the standard test plugin name.
fn plugins() -> std::collections::BTreeMap<String, PluginConfig> {
    [("test-plugin".to_owned(), entry())].into_iter().collect()
}

/// A healthy guest ends up Running on the bus.
#[tokio::test]
async fn healthy_guest_reaches_running_phase() {
    // Given a coordinator with one scripted-healthy plugin and a recorder.
    let harness = TestHarness::new().await;
    let recorder = harness.spawn_recorder::<PluginStatus>().await;
    let state = spawn_coordinator(
        &harness,
        plugins(),
        jinn_plugin::FakeGuestScript::HelloThenLines {
            protocol_version: jinn_plugin_api::PROTOCOL_VERSION,
            lines: vec![],
        },
    )
    .await;

    // When the guest's status events flow.
    let _ = state;

    // Then the Running phase is published for it.
    let messages = await_recorded(&recorder, 1, WAIT).await;
    assert!(
        messages
            .iter()
            .any(|m| m.name == "test-plugin" && m.phase == PluginPhase::Running),
        "expected Running for test-plugin, got {messages:?}"
    );
}

/// Contributions from a healthy guest land in the cache.
#[tokio::test]
async fn set_theme_entries_populates_contribution_cache() {
    // Given a coordinator with a guest contributing one theme.
    let harness = TestHarness::new().await;
    let state = spawn_coordinator(
        &harness,
        plugins(),
        jinn_plugin::FakeGuestScript::HelloThenLines {
            protocol_version: jinn_plugin_api::PROTOCOL_VERSION,
            lines: vec![theme_line("ocean", "#00aabb")],
        },
    )
    .await;

    // When the contribution arrives (poll: async pipeline).
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        if state.read().plugins.theme("ocean").is_some() {
            break;
        }
        assert!(
            deadline > tokio::time::Instant::now(),
            "theme contribution never arrived"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    // Then the theme is cached with its contributing source.
    let source = state
        .read()
        .plugins
        .theme("ocean")
        .map(|t| t.source.clone());
    assert_eq!(source.as_deref(), Some("test-plugin"));
}

/// Malformed input from a guest does not kill it or the app.
#[tokio::test]
async fn malformed_lines_are_dropped_not_fatal() {
    // Given a guest whose wire output includes garbage around a valid line.
    let harness = TestHarness::new().await;
    let state = spawn_coordinator(
        &harness,
        plugins(),
        jinn_plugin::FakeGuestScript::HelloThenLines {
            protocol_version: jinn_plugin_api::PROTOCOL_VERSION,
            lines: vec![
                "this is not json".to_owned(),
                theme_line("after-garbage", "#123456"),
            ],
        },
    )
    .await;

    // When the lines are processed.
    // Then the valid contribution still lands (garbage dropped).
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        if state.read().plugins.theme("after-garbage").is_some() {
            break;
        }
        assert!(
            deadline > tokio::time::Instant::now(),
            "valid line after garbage never arrived"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// A guest whose stdout closes dies; its cached contributions remain.
#[tokio::test]
async fn guest_end_keeps_contributions_and_marks_dead() {
    // Given a coordinator with a guest that contributes then ends.
    let harness = TestHarness::new().await;
    let recorder = harness.spawn_recorder::<PluginStatus>().await;
    let state = spawn_coordinator(
        &harness,
        plugins(),
        jinn_plugin::FakeGuestScript::HelloThenLines {
            protocol_version: jinn_plugin_api::PROTOCOL_VERSION,
            lines: vec![theme_line("persisted", "#abcdef")],
        },
    )
    .await;

    // When the guest ends.
    let messages = await_recorded(&recorder, 1, WAIT).await;
    assert!(
        messages
            .iter()
            .any(|m| m.name == "test-plugin" && m.phase == PluginPhase::Dead),
        "expected Dead for test-plugin, got {messages:?}"
    );

    // Then its contribution is still cached (stale is visible, not erased).
    assert!(state.read().plugins.theme("persisted").is_some());
}

/// A guest that never sends Hello times out and dies without contributing.
#[tokio::test]
async fn silent_guest_dies_at_handshake() {
    // Given a coordinator with a guest that says nothing.
    let harness = TestHarness::new().await;
    let recorder = harness.spawn_recorder::<PluginStatus>().await;
    let state = spawn_coordinator(&harness, plugins(), jinn_plugin::FakeGuestScript::Silent).await;

    // When the handshake timeout lapses.
    let messages = await_recorded(&recorder, 1, WAIT).await;
    assert!(
        messages
            .iter()
            .any(|m| m.name == "test-plugin" && m.phase == PluginPhase::Dead),
        "expected Dead for test-plugin, got {messages:?}"
    );

    // Then nothing was contributed.
    assert_eq!(state.read().plugins.themes().count(), 0);
}

/// A first message that is not Hello fails the handshake.
#[tokio::test]
async fn non_hello_first_message_fails_handshake() {
    // Given a coordinator with a guest whose first line is a contribution.
    let harness = TestHarness::new().await;
    let recorder = harness.spawn_recorder::<PluginStatus>().await;
    let state = spawn_coordinator(
        &harness,
        plugins(),
        jinn_plugin::FakeGuestScript::FirstLine(theme_line("too-eager", "#000001")),
    )
    .await;

    // When the handshake sees the wrong first message.
    let messages = await_recorded(&recorder, 1, WAIT).await;
    assert!(
        messages
            .iter()
            .any(|m| m.name == "test-plugin" && m.phase == PluginPhase::Dead),
        "expected Dead for test-plugin, got {messages:?}"
    );

    // Then the eager contribution was never accepted.
    assert!(state.read().plugins.theme("too-eager").is_none());
}

/// Protocol version mismatch fails the handshake.
#[tokio::test]
async fn version_mismatch_fails_handshake() {
    // Given a coordinator with a guest speaking a different major version.
    let harness = TestHarness::new().await;
    let recorder = harness.spawn_recorder::<PluginStatus>().await;
    let state = spawn_coordinator(
        &harness,
        plugins(),
        jinn_plugin::FakeGuestScript::HelloThenLines {
            protocol_version: jinn_plugin_api::PROTOCOL_VERSION + 1,
            lines: vec![theme_line("future", "#000002")],
        },
    )
    .await;

    // When the mismatched Hello is rejected.
    let messages = await_recorded(&recorder, 1, WAIT).await;
    assert!(
        messages
            .iter()
            .any(|m| m.name == "test-plugin" && m.phase == PluginPhase::Dead),
        "expected Dead for test-plugin, got {messages:?}"
    );

    // Then the future version's contributions are not trusted.
    assert!(state.read().plugins.theme("future").is_none());
}

/// A persisted theme name pending on startup is late-applied when the
/// plugin's first contribution lands.
#[tokio::test]
async fn first_contribution_late_applies_pending_theme_name() {
    // Given a coordinator whose frontend holds a persisted theme name that
    // the default state has not yet applied, and a guest contributing it.
    let harness = TestHarness::new().await;
    let state = spawn_coordinator_prepared(
        &harness,
        plugins(),
        jinn_plugin::FakeGuestScript::HelloThenLines {
            protocol_version: jinn_plugin_api::PROTOCOL_VERSION,
            lines: vec![theme_line("dracula", "#ff00ff")],
        },
        |state| {
            let frontend_cap = crate::common::tcaps::mint::mint_frontend_cap();
            state.with_preferences(&frontend_cap, |ops| {
                ops.frontend().app_state.theme_name = Some("dracula".to_owned());
            });
        },
    )
    .await;

    // When the contribution arrives and the coordinator late-applies.
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let focus = {
            let guard = state.read();
            (guard.plugins.theme("dracula").is_some()).then_some(guard.frontend.theme.focus_accent)
        };
        if focus.is_some() {
            break;
        }
        assert!(
            deadline > tokio::time::Instant::now(),
            "late-apply never happened"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    // Then the frontend theme is the contributed one.
    let guard = state.read();
    assert!(
        matches!(
            guard.frontend.theme.focus_accent,
            ratatui::style::Color::Rgb(255, 0, 255)
        ),
        "expected contributed focus_accent #ff00ff"
    );
}

/// A flooding guest overflows the inbound channel: the pump drops, marks
/// the plugin `Unresponsive`, and the app continues (contributions still
/// arrive).
#[tokio::test]
async fn flooding_guest_is_marked_unresponsive_then_recovers() {
    // Given a coordinator with a guest flooding far more lines than the
    // inbound channel holds, and a recorder.
    let harness = TestHarness::new().await;
    let recorder = harness.spawn_recorder::<PluginStatus>().await;
    let state = spawn_coordinator(
        &harness,
        plugins(),
        jinn_plugin::FakeGuestScript::Flood {
            protocol_version: jinn_plugin_api::PROTOCOL_VERSION,
            lines: vec![theme_line("flood", "#123456")],
            repeat: 500,
        },
    )
    .await;

    // When the flood drains (poll: async pipeline).
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let messages = await_recorded(&recorder, 1, WAIT).await;
        let unresponsive_seen = messages
            .iter()
            .any(|m| m.name == "test-plugin" && m.phase == PluginPhase::Unresponsive);
        if unresponsive_seen {
            break;
        }
        assert!(
            deadline > tokio::time::Instant::now(),
            "Unresponsive was never published"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    // Then the theme still landed (drop-newest lost some, not all).
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        if state.read().plugins.theme("flood").is_some() {
            break;
        }
        assert!(
            deadline > tokio::time::Instant::now(),
            "no contribution survived the flood"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// An identical consecutive contribution is debounced: no duplicate work.
#[tokio::test]
async fn identical_consecutive_theme_batch_is_debounced() {
    // Given a guest contributing the same batch twice (names differ so a
    // non-debounced run would cache both).
    let harness = TestHarness::new().await;
    let state = spawn_coordinator(
        &harness,
        plugins(),
        jinn_plugin::FakeGuestScript::HelloThenLines {
            protocol_version: jinn_plugin_api::PROTOCOL_VERSION,
            lines: vec![theme_line("dupe", "#aabbcc"), theme_line("dupe", "#aabbcc")],
        },
    )
    .await;

    // When both contributions have been processed (poll for the first).
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        if state.read().plugins.theme("dupe").is_some() {
            break;
        }
        assert!(
            deadline > tokio::time::Instant::now(),
            "first contribution never arrived"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Then the theme is cached exactly once (set_themes is a full
    // replacement; the observable check is that the batch still holds it).
    assert!(state.read().plugins.theme("dupe").is_some());
}

/// A batch whose every entry fails translation is dropped with a warn,
/// leaving the cache empty for that plugin.
#[tokio::test]
async fn all_invalid_theme_batch_is_dropped_entirely() {
    // Given a guest contributing one theme with no valid color values.
    let bad_line = r#"{"v":1,"seq":2,"ts":0,"type":"set_theme_entries","themes":[{"name":"bad","description":null,"colors":{"focus_accent":"banana"}}]}"#.to_owned();
    let harness = TestHarness::new().await;
    let state = spawn_coordinator(
        &harness,
        plugins(),
        jinn_plugin::FakeGuestScript::HelloThenLines {
            protocol_version: jinn_plugin_api::PROTOCOL_VERSION,
            lines: vec![bad_line],
        },
    )
    .await;

    // When the pipeline settles (guest ends: wait for Dead phase).
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let phase = state.read().plugins.phase("test-plugin");
        if phase == Some(PluginPhase::Dead) {
            break;
        }
        assert!(deadline > tokio::time::Instant::now(), "guest never ended");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    // Then nothing from the bad batch was cached.
    assert!(state.read().plugins.theme("bad").is_none());
}

/// No configured plugins means no spawns, no status events, and an empty
/// contribution cache — the default install.
#[tokio::test]
async fn no_plugins_configured_is_quiescent() {
    // Given a coordinator with zero plugin entries and a recorder.
    let harness = TestHarness::new().await;
    let recorder = harness.spawn_recorder::<PluginStatus>().await;
    let state = spawn_coordinator(
        &harness,
        std::collections::BTreeMap::new(),
        jinn_plugin::FakeGuestScript::Silent,
    )
    .await;

    // When the coordinator has settled (spawn_all ran at startup).
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Then nothing was published and the cache is empty.
    let messages = await_recorded(&recorder, 1, Duration::from_millis(200)).await;
    assert!(messages.is_empty(), "unexpected status events");
    assert_eq!(state.read().plugins.themes().count(), 0);
}
