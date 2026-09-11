//! Gateway spawn entry point.
//!
//! The discord slice owns its wiring (channels, actors, rows) via
//! `jinn_domain::feat::discord::activate`; this crate is the *frontend* —
//! the one piece that cannot live in the domain: the poise websocket task.
//! Composition (`app.rs`) calls [`spawn_gateway`] once per process; the
//! function decides enablement by reading `[discord] enabled` exactly once
//! and pulling the parked channels from [`Services`] — composition never
//! names a discord type.
//!
//! [`Services`]: jinn_domain::Services

use crate::gateway;
use jinn_domain::Services;
use tokio::task::JoinHandle;

/// Re-exported so callers name one crate for the pool type.
pub use daow::Pool as SessionPool;

/// Spawns the Discord gateway task if `[discord] enabled = true`.
///
/// Reads the enablement gate exactly once. The channels were minted by
/// the slice's `activate()` and are parked on `services.discord`; this
/// pulls the receiving halves and hands the status sender to
/// [`gateway::run`]. When disabled, nothing is spawned and the parked
/// channels stay untouched.
///
/// `session_pool` backs the thread-map DAO; `intent_handler_cap` grants
/// the gateway its God-mode state writes.
pub fn spawn_gateway(
    handle: &tokio::runtime::Handle,
    core: &jinn_domain::AppCore,
    services: &Services,
    session_pool: SessionPool,
    user_preferences_storage: &jinn_domain::UserPreferencesStorageService,
    intent_handler_cap: &jinn_domain::common::tcaps::IntentHandlerCap,
) -> Option<JoinHandle<()>> {
    // The single enablement decision point.
    let config = user_preferences_storage.read().discord.clone();
    if !config.enabled {
        return None;
    }

    let channels = &services.discord;
    let services = services.clone();
    let state = core.state.clone();
    let bridge = core.bridge.clone();
    let intent_handler_cap = *intent_handler_cap;
    let bridge_rx = channels.bridge_rx.clone();
    let gateway_rx = channels.gateway_rx.clone();
    let status_tx = channels.status_tx.clone();
    let token = std::env::var("DISCORD_BOT_TOKEN")
        .ok()
        .or_else(|| config.bot_token.clone())
        .unwrap_or_default();

    Some(handle.spawn(async move {
        if let Err(report) = gateway::run(
            gateway::BotData {
                state,
                bridge,
                thread_map: jinn_domain::feat::discord::DiscordThreadMap::new(session_pool),
                config: std::sync::Arc::new(config),
                services,
                intent_handler_cap,
            },
            token,
            bridge_rx,
            gateway_rx,
            status_tx,
        )
        .await
        {
            tracing::error!("discord gateway terminated: {report:?}");
        }
    }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, reason = "test code")]

    use super::*;
    use jinn_domain::common::bridge::Bridge;
    use jinn_domain::common::state::State;
    use std::sync::Arc;

    /// A throwaway in-memory pool; the disabled path never touches it.
    fn detached_pool() -> SessionPool {
        daow::Pool::builder()
            .path(":memory:")
            .build()
            .expect("in-memory pool")
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn spawn_gateway_noops_when_disabled() {
        // Given services with discord disabled.
        let services = jinn_domain::Services::new_fake().await;
        let mut prefs = services.user_preferences_storage.read();
        prefs.discord.enabled = false;
        services
            .user_preferences_storage
            .save(&prefs)
            .expect("save prefs");
        let bus_actor =
            kameo_actors::message_bus::MessageBus::new(kameo_actors::DeliveryStrategy::BestEffort);
        let bus_ref = kameo::prelude::Spawn::spawn(bus_actor);
        let core = jinn_domain::AppCore {
            state: State::new(jinn_domain::common::app_state::AppState::default()),
            bridge: Bridge::new(bus_ref),
        };
        let prefs_storage = jinn_domain::UserPreferencesStorageService::new(Arc::new(
            jinn_domain::InMemoryUserPreferencesStorage::new(),
        ));
        prefs_storage.reload().expect("test prefs reload");
        let cap = jinn_domain::common::tcaps::mint::mint_intent_handler_cap();

        // When spawning the gateway.
        let handle = spawn_gateway(
            &tokio::runtime::Handle::current(),
            &core,
            &services,
            detached_pool(),
            &prefs_storage,
            &cap,
        );

        // Then no task was spawned.
        assert!(handle.is_none(), "disabled config must not spawn");
    }
}
