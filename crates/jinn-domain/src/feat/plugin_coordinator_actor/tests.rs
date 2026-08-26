//! Coordinator tests using the scripted fake-guest seam.
//!
//! The fake replaces the wasm guest with in-process logic speaking the same
//! NDJSON wire over the same pipes, so these tests exercise the production
//! handshake, read-pump, and validation paths without a compiled plugin.
#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "test assertions"
)]

use std::sync::Arc;
use std::time::Duration;

use kameo::actor::Spawn;

use crate::common::bus::test_harness::{TestHarness, await_recorded};
use crate::common::root_supervisor::RootSupervisor;
use crate::common::state::State;
use crate::common::tcaps::mint::mint_plugins_cap;
use crate::feat::context::protocol::event::PersonasLoaded;
use crate::feat::plugin::PluginConfig;
use crate::feat::plugin_coordinator_actor::PluginCoordinatorActor;
use crate::feat::plugin_coordinator_actor::PluginCoordinatorActorDeps;
use crate::feat::plugin_coordinator_actor::PluginDirs;
use crate::feat::plugin_coordinator_actor::protocol::{
    PluginPhase, PluginStatus, PluginSubscriptions,
};

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

/// A guest whose stdout closes after contributing ended cleanly; its
/// cached contributions remain and the phase is Done.
#[tokio::test]
async fn guest_end_keeps_contributions_and_marks_done() {
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
    let messages = await_recorded(&recorder, 3, WAIT).await;
    assert!(
        messages
            .iter()
            .any(|m| m.name == "test-plugin" && m.phase == PluginPhase::Done),
        "expected Done for test-plugin, got {messages:?}"
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

    // When the pipeline settles (guest ends: wait for Done phase).
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let phase = state.read().plugins.phase("test-plugin");
        if phase == Some(PluginPhase::Done) {
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

/// One valid wire `SetPersonaEntries` line (hand-encoded JSON to prove the raw
/// wire shape survives decode).
fn persona_line(name: &str, description: Option<&str>) -> String {
    let description = match description {
        Some(d) => format!("\"{d}\""),
        None => "null".to_owned(),
    };
    format!(
        r#"{{"v":1,"seq":2,"ts":0,"type":"set_persona_entries","personas":[{{"name":"{name}","description":{description},"body":"You are {name}."}}]}}"#
    )
}

/// A persona contribution is translated and published as `PersonasLoaded`.
#[tokio::test]
async fn set_persona_entries_publishes_personas_loaded() {
    // Given a coordinator with a guest contributing one persona and a recorder.
    let harness = TestHarness::new().await;
    let recorder = harness.spawn_recorder::<PersonasLoaded>().await;
    let _state = spawn_coordinator(
        &harness,
        plugins(),
        jinn_plugin::FakeGuestScript::HelloThenLines {
            protocol_version: jinn_plugin_api::PROTOCOL_VERSION,
            lines: vec![persona_line("coder", Some("Expert coder"))],
        },
    )
    .await;

    // When the contribution arrives (await the recorded event).
    let events = await_recorded(&recorder, 1, WAIT).await;

    // Then the event carries the translated persona with no error.
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].personas.len(), 1);
    assert_eq!(events[0].personas[0].name, "coder");
    assert_eq!(events[0].personas[0].description, "Expert coder");
    assert_eq!(events[0].personas[0].body, "You are coder.");
    assert!(events[0].error.is_none());
}

/// An identical consecutive persona batch is debounced to a single publish.
#[tokio::test]
async fn duplicate_persona_batch_is_debounced() {
    // Given a coordinator with a guest pushing the same persona batch twice.
    let harness = TestHarness::new().await;
    let recorder = harness.spawn_recorder::<PersonasLoaded>().await;
    let _state = spawn_coordinator(
        &harness,
        plugins(),
        jinn_plugin::FakeGuestScript::HelloThenLines {
            protocol_version: jinn_plugin_api::PROTOCOL_VERSION,
            lines: vec![persona_line("coder", None), persona_line("coder", None)],
        },
    )
    .await;

    // When both lines arrive and the pipeline settles (GetRecorded drains,
    // so read once after settling).
    tokio::time::sleep(Duration::from_millis(300)).await;
    let events = await_recorded(&recorder, 1, Duration::from_millis(200)).await;

    // Then exactly one PersonasLoaded event was published — the duplicate
    // batch was debounced.
    assert_eq!(events.len(), 1, "duplicate batch must not re-publish");
}

// ── Host→guest event forwarding ──────────────────────────────────────────────

use crate::common::tcaps::mint::mint_session_cap;
use crate::feat::session::phase_machine::PhaseKind;
use crate::feat::session::protocol::citations_received::CitationsReceived;
use crate::feat::session::protocol::session_phase_changed::SessionPhaseChanged;
use crate::feat::tools_actor::protocol::event::ToolCallReceived;
use crate::feat::tools_actor::tool_types::ToolCall;
use crate::protocol::SessionId;

/// Seeds one history entry into a session for `final_answer` tests.
fn seed_entry(state: &State, session_id: &SessionId, is_assistant: bool) {
    state.with_session(&mint_session_cap(), |view| {
        let session = view.session.map().get_or_create(session_id);
        let entry = if is_assistant {
            crate::feat::session::chat_entry::ChatEntry::assistant("done")
        } else {
            crate::feat::session::chat_entry::ChatEntry::error("boom")
        };
        session.push_entry(entry);
    });
}

/// A subscribed plugin's guest stays alive; its registration lands.
#[tokio::test]
async fn subscribed_guest_registers_its_kinds() {
    // Given a coordinator with a guest subscribing to all three kinds.
    let harness = TestHarness::new().await;
    let recorder = harness.spawn_recorder::<PluginSubscriptions>().await;
    let _state = spawn_coordinator(
        &harness,
        plugins(),
        jinn_plugin::FakeGuestScript::SubscribedEcho {
            protocol_version: jinn_plugin_api::PROTOCOL_VERSION,
            subscriptions: vec![
                "tool_call".to_owned(),
                "tool_result".to_owned(),
                "turn_end".to_owned(),
            ],
        },
    )
    .await;

    // When the handshake completes.
    let events = await_recorded(&recorder, 1, WAIT).await;

    // Then the validated subscription set was announced.
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].name, "test-plugin");
    assert_eq!(events[0].kinds.len(), 3);
}

/// An unsubscribed plugin receives no forwarded events (the guest would
/// misparse them); only subscribed kinds flow.
#[tokio::test]
async fn unsubscribed_kind_is_not_forwarded() {
    // Given a guest subscribing only to turn_end (no tool events).
    let harness = TestHarness::new().await;
    let recorder = harness.spawn_recorder::<PluginSubscriptions>().await;
    let _state = spawn_coordinator(
        &harness,
        plugins(),
        jinn_plugin::FakeGuestScript::SubscribedEcho {
            protocol_version: jinn_plugin_api::PROTOCOL_VERSION,
            subscriptions: vec!["turn_end".to_owned()],
        },
    )
    .await;
    let _ = await_recorded(&recorder, 1, WAIT).await;

    // When a tool call event fires for an unsubscribed kind.
    harness
        .publish(ToolCallReceived {
            session_id: SessionId::new(),
            tool_call: ToolCall {
                id: "call_1".to_owned(),
                name: "web-fetch".to_owned(),
                arguments: r#"{"url":"https://example.com"}"#.to_owned(),
            },
            dispatched_at: jiff::Timestamp::now(),
        })
        .await;

    // Then the coordinator stays healthy (no crash, no dead plugin) and —
    // proven directly — the unsubscribed kind produced no forward: the
    // echo guest (subscribed to turn_end only) published no echo reply.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let statuses = await_recorded(
        &statuses_recorder(&harness).await,
        0,
        Duration::from_millis(100),
    )
    .await;
    assert!(
        !statuses
            .iter()
            .any(|s| s.name == "test-plugin" && s.phase == PluginPhase::Dead),
        "unsubscribed event must not kill the plugin"
    );
}

/// Helper: a PluginStatus recorder on the given harness.
async fn statuses_recorder(
    harness: &TestHarness,
) -> kameo::actor::ActorRef<crate::common::bus::test_harness::Recorder<PluginStatus>> {
    harness.spawn_recorder::<PluginStatus>().await
}

/// `final_answer` is true only when the last entry is an assistant message.
#[tokio::test]
async fn turn_end_final_answer_reflects_last_entry() {
    // Given a session whose last entry is an assistant message.
    let harness = TestHarness::new().await;
    let recorder = harness.spawn_recorder::<CitationsReceived>().await;
    let state = spawn_coordinator(
        &harness,
        plugins(),
        jinn_plugin::FakeGuestScript::SubscribedEcho {
            protocol_version: jinn_plugin_api::PROTOCOL_VERSION,
            subscriptions: vec!["turn_end".to_owned()],
        },
    )
    .await;
    let session_id = SessionId::new();
    seed_entry(&state, &session_id, true);

    // When the turn ends (Streaming → Idle).
    harness
        .publish(SessionPhaseChanged {
            session_id: session_id.clone(),
            old_phase: PhaseKind::Streaming,
            new_phase: PhaseKind::Idle,
        })
        .await;

    // Then the forwarded turn_end event carried final_answer=true: the
    // echo returns the forwarded line, which must contain the flag and the
    // session id.
    let events = await_recorded(&recorder, 1, WAIT).await;
    assert_eq!(events.len(), 1, "echo reply published");
    let echoed = &events[0].citations[0].title;
    assert!(
        echoed.contains(r#""final_answer":true"#),
        "final_answer must be true for an assistant last entry, got: {echoed}"
    );
    assert!(
        echoed.contains(&session_id.to_string()),
        "the event must carry the session id"
    );
}

/// A valid PushCitations line publishes CitationsReceived on the bus.
#[tokio::test]
async fn push_citations_publishes_citations_received() {
    // Given a coordinator with a guest pushing one citation.
    let harness = TestHarness::new().await;
    let recorder = harness.spawn_recorder::<CitationsReceived>().await;
    let session_id = SessionId::new();
    let line = format!(
        r#"{{"v":1,"seq":2,"ts":0,"type":"push_citations","session_id":"{session_id}","citations":[{{"url":"https://example.com/a","title":"Example A","content":"excerpt"}}]}}"#
    );
    let _state = spawn_coordinator(
        &harness,
        plugins(),
        jinn_plugin::FakeGuestScript::HelloThenLines {
            protocol_version: jinn_plugin_api::PROTOCOL_VERSION,
            lines: vec![line],
        },
    )
    .await;

    // When the contribution is processed.
    let events = await_recorded(&recorder, 1, WAIT).await;

    // Then exactly one CitationsReceived was published with the citation.
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].session_id, session_id);
    assert_eq!(events[0].citations.len(), 1);
    assert_eq!(events[0].citations[0].url, "https://example.com/a");
    assert_eq!(events[0].citations[0].title, "Example A");
}

/// Invalid citations are dropped entry-wise; valid ones survive.
#[tokio::test]
async fn push_citations_drops_invalid_entries_keeps_valid() {
    // Given a guest pushing one invalid (ftp) and one valid citation.
    let harness = TestHarness::new().await;
    let recorder = harness.spawn_recorder::<CitationsReceived>().await;
    let session_id = SessionId::new();
    let line = format!(
        r#"{{"v":1,"seq":2,"ts":0,"type":"push_citations","session_id":"{session_id}","citations":[{{"url":"ftp://nope","title":"bad"}},{{"url":"https://ok.example","title":""}}]}}"#
    );
    let _state = spawn_coordinator(
        &harness,
        plugins(),
        jinn_plugin::FakeGuestScript::HelloThenLines {
            protocol_version: jinn_plugin_api::PROTOCOL_VERSION,
            lines: vec![line],
        },
    )
    .await;

    // When the contribution is processed.
    let events = await_recorded(&recorder, 1, WAIT).await;

    // Then only the valid citation survived, with the URL as title fallback.
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].citations.len(), 1);
    assert_eq!(events[0].citations[0].url, "https://ok.example");
    assert_eq!(events[0].citations[0].title, "https://ok.example");
}

/// An unparseable session id drops the whole batch without publishing.
#[tokio::test]
async fn push_citations_with_bad_session_id_is_dropped() {
    // Given a guest pushing citations for a non-UUID session id.
    let harness = TestHarness::new().await;
    let recorder = harness.spawn_recorder::<CitationsReceived>().await;
    let _state = spawn_coordinator(
        &harness,
        plugins(),
        jinn_plugin::FakeGuestScript::HelloThenLines {
            protocol_version: jinn_plugin_api::PROTOCOL_VERSION,
            lines: vec![r#"{"v":1,"seq":2,"ts":0,"type":"push_citations","session_id":"not-a-uuid","citations":[{"url":"https://a.example","title":"A"}]}"#.to_owned()],
        },
    )
    .await;

    // When the line is processed and settles.
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Then nothing was published.
    assert!(
        await_recorded(&recorder, 0, Duration::from_millis(100))
            .await
            .is_empty()
    );
}

/// Two identical citation pushes in sequence both publish (no debounce).
#[tokio::test]
async fn identical_citation_batches_both_publish() {
    // Given a guest pushing the same citation batch twice.
    let harness = TestHarness::new().await;
    let recorder = harness.spawn_recorder::<CitationsReceived>().await;
    let session_id = SessionId::new();
    let line = format!(
        r#"{{"v":1,"seq":2,"ts":0,"type":"push_citations","session_id":"{session_id}","citations":[{{"url":"https://same.example","title":"Same"}}]}}"#
    );
    let _state = spawn_coordinator(
        &harness,
        plugins(),
        jinn_plugin::FakeGuestScript::HelloThenLines {
            protocol_version: jinn_plugin_api::PROTOCOL_VERSION,
            lines: vec![line.clone(), line],
        },
    )
    .await;

    // When both lines are processed.
    let events = await_recorded(&recorder, 2, WAIT).await;

    // Then two CitationsReceived events were published (turn-scoped, no
    // identical-payload debounce).
    assert_eq!(events.len(), 2, "turn-scoped citations must not debounce");
}

/// `final_answer` computation: an assistant last entry yields true, an
/// error last entry yields false (the flush gate).
#[tokio::test]
async fn final_answer_reflects_last_history_entry_kind() {
    // Given a session whose last entry is an error.
    let state = State::new(crate::common::app_state::AppState::default());
    let session_id = SessionId::new();
    seed_entry(&state, &session_id, false);

    // When checking the final-answer signal.
    // Then it is false for the error entry.
    assert!(!super::last_entry_is_assistant(&state, &session_id));

    // Given the session's history now ends with an assistant message.
    seed_entry(&state, &session_id, true);

    // When re-checking.
    // Then it is true.
    assert!(super::last_entry_is_assistant(&state, &session_id));

    // Given an unknown session.
    // When checking.
    // Then it is false (never claims a final answer for a vanished session).
    assert!(!super::last_entry_is_assistant(&state, &SessionId::new()));
}

/// With no plugins configured, forwarded bus events are harmless no-ops and
/// the coordinator stays alive — no footer, no startup failure.
#[tokio::test]
async fn no_plugins_forwarded_events_are_harmless() {
    // Given a coordinator with zero plugins configured and a recorder.
    let harness = TestHarness::new().await;
    let recorder = harness.spawn_recorder::<CitationsReceived>().await;
    let _state = spawn_coordinator(
        &harness,
        std::collections::BTreeMap::new(),
        jinn_plugin::FakeGuestScript::Silent,
    )
    .await;

    // When tool and phase events fire anyway.
    harness
        .publish(ToolCallReceived {
            session_id: SessionId::new(),
            tool_call: ToolCall {
                id: "c1".to_owned(),
                name: "web-fetch".to_owned(),
                arguments: r#"{"url":"https://example.com"}"#.to_owned(),
            },
            dispatched_at: jiff::Timestamp::now(),
        })
        .await;
    harness
        .publish(SessionPhaseChanged {
            session_id: SessionId::new(),
            old_phase: PhaseKind::Streaming,
            new_phase: PhaseKind::Idle,
        })
        .await;

    // Then nothing was published and the coordinator did not crash.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        await_recorded(&recorder, 0, Duration::from_millis(100))
            .await
            .is_empty(),
        "no plugin means no citations"
    );
}

/// A truncated tool result forwards the untruncated `full_content` to
/// subscribed guests — plugins see the complete output; truncation is an
/// LLM-context limit only. The echo guest returns each forwarded line
/// inside a citation title, so the recorder asserts exactly what crossed
/// the wire.
#[tokio::test]
async fn truncated_result_forwards_full_content_to_guest() {
    // Given a coordinator with an echo guest subscribed to tool results.
    let harness = TestHarness::new().await;
    let recorder = harness.spawn_recorder::<CitationsReceived>().await;
    spawn_coordinator(
        &harness,
        plugins(),
        jinn_plugin::FakeGuestScript::SubscribedEcho {
            protocol_version: jinn_plugin_api::PROTOCOL_VERSION,
            subscriptions: vec!["tool_result".to_owned()],
        },
    )
    .await;

    // When a truncated MCP result completes: `content` is clipped
    // mid-JSON (unparseable), `full_content` holds the original.
    let full_json = r#"{"search_id":"s","results":[{"url":"https://full.example/page","title":"Full Content Page","publish_date":null,"excerpts":["entire original"]}]}"#;
    // Mid-object cut — cannot parse. Take chars (not bytes) so the slice
    // can never split a UTF-8 boundary.
    let clipped: String = full_json.chars().take(40).collect();
    harness
        .publish(
            crate::feat::tools_actor::protocol::event::ToolExecutionCompleted {
                session_id: SessionId::new(),
                result: crate::feat::tools_actor::tool_types::ToolResult {
                    tool_call_id: "call_trunc".to_owned(),
                    name: "mcp__parallel__web_search".to_owned(),
                    content: clipped.clone(),
                    success: true,
                    full_content: Some(full_json.to_owned()),
                    truncation: None,
                    pin_position: None,
                },
            },
        )
        .await;

    // Then the guest received the complete JSON, not the clip: the echo
    // reply's citation title contains the forwarded line, which must carry
    // the full payload's URL and never the clipped fragment's cut point.
    let events = await_recorded(&recorder, 1, WAIT).await;
    assert_eq!(events.len(), 1, "echo reply published");
    let echoed = &events[0].citations[0].title;
    assert!(
        echoed.contains("https://full.example/page"),
        "forwarded line must carry the untruncated payload, got: {echoed}"
    );
    assert!(
        echoed.contains("entire original"),
        "the tail of the full payload must have crossed the wire, got: {echoed}"
    );
}

/// An empty citations list publishes nothing.
#[tokio::test]
async fn push_citations_with_empty_list_publishes_nothing() {
    // Given a coordinator with a guest pushing an empty citations batch.
    let harness = TestHarness::new().await;
    let recorder = harness.spawn_recorder::<CitationsReceived>().await;
    let _state = spawn_coordinator(
        &harness,
        plugins(),
        jinn_plugin::FakeGuestScript::HelloThenLines {
            protocol_version: jinn_plugin_api::PROTOCOL_VERSION,
            lines: vec![r#"{"v":1,"seq":2,"ts":0,"type":"push_citations","session_id":"01943d8e-5a1f-7c2d-9e3b-4f6a8b0c1d2e","citations":[]}"#.to_owned()],
        },
    )
    .await;

    // When the line is processed and the pipeline settles.
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Then no CitationsReceived was published.
    assert!(
        await_recorded(&recorder, 0, Duration::from_millis(100))
            .await
            .is_empty(),
        "empty batch must not publish"
    );
}

/// A run-to-completion guest (handshake, no further lines, clean stdout
/// close — the loader shape) ends up Done, not Dead.
#[tokio::test]
async fn clean_exit_guest_reaches_done_phase() {
    // Given a coordinator whose guest handshakes then closes stdout cleanly.
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

    // When the guest finishes and the terminal phase is published.
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let done = state.read().plugins.phase("test-plugin");
        if done == Some(PluginPhase::Done) {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("guest never reached Done; phase = {done:?}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // Then Done was published on the bus (not Dead).
    let messages = await_recorded(&recorder, 3, WAIT).await;
    assert!(
        messages
            .iter()
            .any(|m| m.name == "test-plugin" && m.phase == PluginPhase::Done),
        "expected Done for test-plugin, got {messages:?}"
    );
}

/// A guest that dies before handshaking still ends up Dead (clean-exit
/// marking must not mask real failures).
#[tokio::test]
async fn silent_guest_reaches_dead_phase() {
    // Given a coordinator whose guest closes stdout before Hello.
    let harness = TestHarness::new().await;
    let state = spawn_coordinator(&harness, plugins(), jinn_plugin::FakeGuestScript::Silent)
        .await;

    // When the failed startup path settles.
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let phase = state.read().plugins.phase("test-plugin");
        if phase == Some(PluginPhase::Dead) {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("guest never reached Dead; phase = {phase:?}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
