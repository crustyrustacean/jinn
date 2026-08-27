//! Tests for the discord feature's domain-layer pieces.
//!
//! Covers:
//! - `DiscordConfig` serde defaults (empty TOML → disabled bot)
//! - `DiscordThreadMap` DAO round-trip (set + both lookups + unknown→None)

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::indexing_slicing,
    reason = "test code"
)]

use crate::feat::discord::{DiscordConfig, DiscordThreadMap};
use serde::Deserialize;

#[rstest::rstest]
#[test]
fn empty_toml_deserializes_to_disabled_bot() {
    // Given an empty TOML document.
    let toml_str = "";

    // When deserializing as a DiscordConfig.
    let cfg: DiscordConfig = toml::from_str(toml_str).expect("parse");

    // Then the bot is disabled with no token/guild.
    assert!(!cfg.enabled);
    assert_eq!(cfg.bot_token, None);
    assert_eq!(cfg.guild_id, None);
}

#[rstest::rstest]
#[test]
fn populated_discord_table_round_trips() {
    // Given a populated [discord] table.
    #[derive(Deserialize)]
    struct Wrapper {
        discord: DiscordConfig,
    }
    let toml_str = r#"
        [discord]
        enabled = true
        bot_token = "abc123"
        guild_id = "9999"
    "#;

    // When deserializing.
    let parsed: Wrapper = toml::from_str(toml_str).expect("parse");

    // Then all fields are preserved.
    assert!(parsed.discord.enabled);
    assert_eq!(parsed.discord.bot_token.as_deref(), Some("abc123"));
    assert_eq!(parsed.discord.guild_id.as_deref(), Some("9999"));
}

#[rstest::rstest]
#[test]
fn leftover_lifecycle_key_is_ignored() {
    // Given a [discord] table with a stale `lifecycle` key plus the
    // current fields.
    #[derive(Deserialize)]
    struct Wrapper {
        discord: DiscordConfig,
    }
    let toml_str = r#"
        [discord]
        enabled = true
        bot_token = "abc123"
        lifecycle = "x"
        guild_id = "9999"
    "#;

    // When deserializing.
    let parsed: Wrapper = toml::from_str(toml_str).expect("parse");

    // Then the stale `lifecycle` key is silently dropped and the
    // remaining fields are populated.
    assert!(parsed.discord.enabled);
    assert_eq!(parsed.discord.bot_token.as_deref(), Some("abc123"));
    assert_eq!(parsed.discord.guild_id.as_deref(), Some("9999"));
}

#[rstest::rstest]
#[test]
fn re_serializing_disabled_default_round_trips() {
    // Given a default config.
    let cfg = DiscordConfig::default();

    // When serializing then re-parsing.
    let s = toml::to_string(&cfg).expect("serialize");
    let reparsed: DiscordConfig = toml::from_str(&s).expect("reparse");

    // Then it equals the original (still disabled).
    assert_eq!(cfg, reparsed);
    assert!(!reparsed.enabled);
}

// ── DiscordThreadMap DAO ──────────────────────────────────────────────

use crate::feat::session::session_store::SqliteSessionStore;
use tempfile::TempDir;

async fn make_map() -> (TempDir, DiscordThreadMap) {
    let dir = TempDir::new().expect("temp dir");
    let store = SqliteSessionStore::new_in(dir.path()).await.expect("store");
    let map = DiscordThreadMap::new(store.pool().clone());
    (dir, map)
}

#[rstest::rstest]
#[tokio::test]
async fn dao_set_then_forward_lookup_returns_session_id() {
    // Given an empty map with one mapping inserted.
    let (_dir, map) = make_map().await;
    map.set("thread-1", "session-1", Some("guild-1"), 1_700_000_000)
        .await
        .expect("set");

    // When doing the forward lookup.
    let session = map.get_session_by_thread("thread-1").await.expect("lookup");

    // Then the stored session id is returned.
    assert_eq!(session.as_deref(), Some("session-1"));
}

#[rstest::rstest]
#[tokio::test]
async fn dao_set_then_reverse_lookup_returns_mapping() {
    // Given a map with one mapping.
    let (_dir, map) = make_map().await;
    map.set("thread-1", "session-1", Some("guild-1"), 1_700_000_000)
        .await
        .expect("set");

    // When doing the reverse lookup.
    let mapping = map
        .get_thread_by_session("session-1")
        .await
        .expect("lookup");

    // Then the full mapping is returned.
    let mapping = mapping.expect("mapping exists");
    assert_eq!(mapping.thread_id, "thread-1");
    assert_eq!(mapping.session_id, "session-1");
    assert_eq!(mapping.guild_id.as_deref(), Some("guild-1"));
    assert_eq!(mapping.created_at, 1_700_000_000);
}

#[rstest::rstest]
#[tokio::test]
async fn dao_unknown_thread_returns_none() {
    // Given an empty map.
    let (_dir, map) = make_map().await;

    // When looking up a thread that was never recorded.
    let session = map
        .get_session_by_thread("never-seen")
        .await
        .expect("lookup");

    // Then None is returned.
    assert!(session.is_none());
}

#[rstest::rstest]
#[tokio::test]
async fn dao_set_rebinds_existing_thread_to_new_session() {
    // Given a thread already mapped to session-1.
    let (_dir, map) = make_map().await;
    map.set("thread-1", "session-1", None, 1)
        .await
        .expect("first set");

    // When re-running set for the same thread with a new session.
    map.set("thread-1", "session-2", None, 2)
        .await
        .expect("rebind");

    // Then the forward lookup returns the new session.
    let session = map.get_session_by_thread("thread-1").await.expect("lookup");
    assert_eq!(session.as_deref(), Some("session-2"));
}

// ── DiscordBridgeActor forwarding ─────────────────────────────

use crate::common::app_state::AppState;
use crate::common::state::State;
use crate::feat::discord::bridge_actor::DiscordBridgeActor;
use crate::feat::discord::protocol::BridgeEvent;
use crate::feat::session::phase_machine::PhaseKind;
use crate::feat::session::protocol::session_archived::SessionArchived;
use crate::feat::session::protocol::session_phase_changed::SessionPhaseChanged;
use crate::feat::session_lifecycle::protocol::event::{
    SessionSetupCompleted, SessionTeardownFinished,
};
use crate::protocol::SessionId;
use std::path::PathBuf;

fn session_id() -> SessionId {
    SessionId::new()
}

fn make_actor() -> (DiscordBridgeActor, kanal::AsyncReceiver<BridgeEvent>) {
    let (tx, rx) = kanal::bounded(64);
    (
        DiscordBridgeActor::new(
            tx,
            State::new(AppState::default()),
            crate::common::tcaps::mint::mint_session_cap(),
        ),
        rx.to_async(),
    )
}

#[rstest::rstest]
#[tokio::test]
async fn idle_transition_forwards_one_turn_finished() {
    // Given a bridge actor.
    let (actor, rx) = make_actor();
    let sid = session_id();

    // When receiving a SessionPhaseChanged → Idle.
    actor.handle_session_phase_changed(&SessionPhaseChanged {
        session_id: sid.clone(),
        old_phase: PhaseKind::Streaming,
        new_phase: PhaseKind::Idle,
    });

    // Then exactly one TurnFinished was forwarded.
    let event = rx.recv().await.expect("event forwarded");
    match event {
        BridgeEvent::TurnFinished { session_id } => {
            assert_eq!(session_id, sid);
        }
        other => panic!("expected TurnFinished, got {other:?}"),
    }
    assert!(matches!(rx.try_recv(), Ok(None)), "no extra events");
}

#[rstest::rstest]
#[tokio::test]
async fn streaming_transition_is_dropped() {
    // Given a bridge actor.
    let (actor, rx) = make_actor();

    // When receiving a non-Idle phase transition.
    actor.handle_session_phase_changed(&SessionPhaseChanged {
        session_id: session_id(),
        old_phase: PhaseKind::Idle,
        new_phase: PhaseKind::Streaming,
    });

    // Then no event was forwarded.
    assert!(
        matches!(rx.try_recv(), Ok(None)),
        "non-Idle transition must not forward"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn setup_completed_forwards_cwd_and_error() {
    // Given a bridge actor.
    let (actor, rx) = make_actor();
    let sid = session_id();

    // When receiving a SessionSetupCompleted.
    actor.handle_session_setup_completed(&SessionSetupCompleted {
        session_id: sid.clone(),
        cwd: PathBuf::from("/repo"),
        error: Some("boom".to_owned()),
    });

    // Then exactly one SetupCompleted was forwarded, carrying the payload.
    let event = rx.recv().await.expect("event forwarded");
    match event {
        BridgeEvent::SetupCompleted {
            session_id,
            cwd,
            error,
        } => {
            assert_eq!(session_id, sid);
            assert_eq!(cwd, PathBuf::from("/repo"));
            assert_eq!(error.as_deref(), Some("boom"));
        }
        other => panic!("expected SetupCompleted, got {other:?}"),
    }
    assert!(matches!(rx.try_recv(), Ok(None)), "no extra events");
}

#[rstest::rstest]
#[tokio::test]
async fn teardown_finished_forwards_error_and_session() {
    // Given a bridge actor.
    let (actor, rx) = make_actor();
    let sid = session_id();

    // When receiving a failed SessionTeardownFinished.
    actor.handle_session_teardown_finished(&SessionTeardownFinished {
        session_id: sid.clone(),
        error: Some("boom".to_owned()),
    });

    // Then exactly one TeardownFinished was forwarded, carrying the payload.
    let event = rx.recv().await.expect("event forwarded");
    match event {
        BridgeEvent::TeardownFinished { session_id, error } => {
            assert_eq!(session_id, sid);
            assert_eq!(error.as_deref(), Some("boom"));
        }
        other => panic!("expected TeardownFinished, got {other:?}"),
    }
    assert!(matches!(rx.try_recv(), Ok(None)), "no extra events");
}

#[rstest::rstest]
#[tokio::test]
async fn archived_forwards_session_id() {
    // Given a bridge actor.
    let (actor, rx) = make_actor();
    let sid = session_id();

    // When receiving a SessionArchived.
    actor.handle_session_archived(&SessionArchived {
        session_id: sid.clone(),
    });

    // Then exactly one Archived was forwarded, carrying the session id.
    let event = rx.recv().await.expect("event forwarded");
    match event {
        BridgeEvent::Archived { session_id } => {
            assert_eq!(session_id, sid);
        }
        other => panic!("expected Archived, got {other:?}"),
    }
    assert!(matches!(rx.try_recv(), Ok(None)), "no extra events");
}

// ── Bus-spawned subscription integration tests ────────────────────
//
// The unit tests above call the handle_* forwarders directly via the
// `new(tx)` constructor. They prove the forwarding logic works but NOT
// that `on_start` actually subscribes to each event type on the bus.
// A missing `subscribe` call (the bug this feature fixes) would pass
// every one of those tests. The tests below spawn the actor via `on_start`
// against a real bus and publish events on it, so they fail if any
// subscription is dropped.

use crate::common::actor_deps::ActorDeps;
use crate::common::bus::test_harness::TestHarness;
use crate::feat::discord::{
    CreateThreadForSession, GatewayRequest, bridge_actor::DiscordBridgeActorDeps,
};
async fn spawn_on_bus_with_rx() -> (
    TestHarness,
    kanal::AsyncReceiver<BridgeEvent>,
    kanal::AsyncReceiver<GatewayRequest>,
) {
    let harness = TestHarness::new().await;
    let (tx, rx) = kanal::bounded(64);
    let deps = ActorDeps {
        services: harness.services().await,
    };
    let (gw_tx, gw_rx) = kanal::bounded(16);
    let _actor = harness
        .spawn_actor::<DiscordBridgeActor>(DiscordBridgeActorDeps {
            deps,
            tx,
            gateway_tx: gw_tx,
            state: State::new(AppState::default()),
            session_cap: crate::common::tcaps::mint::mint_session_cap(),
        })
        .await;
    (harness, rx.to_async(), gw_rx.to_async())
}

#[rstest::rstest]
#[tokio::test]
async fn bus_subscription_forwards_turn_finished() {
    // Given a bridge actor spawned via on_start against a real bus.
    let (harness, rx, _gw_rx) = spawn_on_bus_with_rx().await;
    let sid = session_id();

    // When publishing a SessionPhaseChanged → Idle on the bus.
    harness
        .publish(SessionPhaseChanged {
            session_id: sid.clone(),
            old_phase: PhaseKind::Streaming,
            new_phase: PhaseKind::Idle,
        })
        .await;

    // Then exactly one TurnFinished was forwarded.
    let event = rx.recv().await.expect("event forwarded");
    match event {
        BridgeEvent::TurnFinished { session_id } => {
            assert_eq!(session_id, sid);
        }
        other => panic!("expected TurnFinished, got {other:?}"),
    }
    assert!(matches!(rx.try_recv(), Ok(None)), "no extra events");
}

#[rstest::rstest]
#[tokio::test]
async fn bus_subscription_forwards_setup_completed() {
    // Given a bridge actor spawned via on_start against a real bus.
    let (harness, rx, _gw_rx) = spawn_on_bus_with_rx().await;
    let sid = session_id();

    // When publishing a SessionSetupCompleted on the bus.
    harness
        .publish(SessionSetupCompleted {
            session_id: sid.clone(),
            cwd: PathBuf::from("/repo"),
            error: Some("boom".to_owned()),
        })
        .await;

    // Then exactly one SetupCompleted was forwarded, carrying the payload.
    let event = rx.recv().await.expect("event forwarded");
    match event {
        BridgeEvent::SetupCompleted {
            session_id,
            cwd,
            error,
        } => {
            assert_eq!(session_id, sid);
            assert_eq!(cwd, PathBuf::from("/repo"));
            assert_eq!(error.as_deref(), Some("boom"));
        }
        other => panic!("expected SetupCompleted, got {other:?}"),
    }
    assert!(matches!(rx.try_recv(), Ok(None)), "no extra events");
}

#[rstest::rstest]
#[tokio::test]
async fn bus_subscription_forwards_teardown_finished() {
    // Given a bridge actor spawned via on_start against a real bus.
    let (harness, rx, _gw_rx) = spawn_on_bus_with_rx().await;
    let sid = session_id();

    // When publishing a failed SessionTeardownFinished on the bus.
    harness
        .publish(SessionTeardownFinished {
            session_id: sid.clone(),
            error: Some("boom".to_owned()),
        })
        .await;

    // Then exactly one TeardownFinished was forwarded, carrying the payload.
    let event = rx.recv().await.expect("event forwarded");
    match event {
        BridgeEvent::TeardownFinished { session_id, error } => {
            assert_eq!(session_id, sid);
            assert_eq!(error.as_deref(), Some("boom"));
        }
        other => panic!("expected TeardownFinished, got {other:?}"),
    }
    assert!(matches!(rx.try_recv(), Ok(None)), "no extra events");
}

#[rstest::rstest]
#[tokio::test]
async fn bus_subscription_forwards_archived() {
    // Given a bridge actor spawned via on_start against a real bus.
    let (harness, rx, _gw_rx) = spawn_on_bus_with_rx().await;
    let sid = session_id();

    // When publishing a SessionArchived on the bus.
    harness
        .publish(SessionArchived {
            session_id: sid.clone(),
        })
        .await;

    // Then exactly one Archived was forwarded, carrying the session id.
    let event = rx.recv().await.expect("event forwarded");
    match event {
        BridgeEvent::Archived { session_id } => {
            assert_eq!(session_id, sid);
        }
        other => panic!("expected Archived, got {other:?}"),
    }
    assert!(matches!(rx.try_recv(), Ok(None)), "no extra events");
}

#[rstest::rstest]
#[tokio::test]
async fn bus_subscription_forwards_create_thread_for_session() {
    // Given a bridge actor spawned via on_start against a real bus.
    let (harness, _rx, gw_rx) = spawn_on_bus_with_rx().await;
    let sid = session_id();

    // When publishing CreateThreadForSession on the bus.
    harness
        .publish(CreateThreadForSession {
            session_id: sid.clone(),
            title: "my thread".to_owned(),
        })
        .await;

    // Then exactly one GatewayRequest::CreateThreadForSession was forwarded,
    // carrying the payload.
    let request = gw_rx.recv().await.expect("request forwarded");
    let GatewayRequest::CreateThreadForSession { session_id, title } = request;
    assert_eq!(session_id, sid);
    assert_eq!(title, "my thread");
    assert!(matches!(gw_rx.try_recv(), Ok(None)), "no extra requests");
}
