//! Discord's slice activation — the single composition seam.
//!
//! One call from composition (`actor_wiring::build`) is the slice's
//! entire integration surface: mint the connection cell, spawn the
//! status actor (always — it is the connection authority regardless of
//! bridge enablement), create all three gateway kanal channels
//! unconditionally, spawn the bridge actor only when `[discord]
//! enabled = true`, park the gateway-facing channel halves on
//! `Services`, and attach the route rows.
//!
//! The `enabled` gate is *not* collapsed into the bridge spawn: the
//! to-thread precondition chain re-checks it from state, and the
//! gateway frontend re-checks it before starting the websocket. What
//! this function guarantees is the wiring topology — every channel
//! exists, exactly one actor feeds or drains each, and removing this
//! call removes discord with no other edits.

use kameo::actor::Spawn;

use super::DiscordStatusActor;
use super::DiscordStatusActorDeps;
use super::channels::DiscordGatewayChannels;
use super::channels::MintedChannels;
use super::status_actor::ConnectionState;
use crate::Services;
use crate::common::actor_deps::ActorDeps;
use crate::common::state::State;

/// Activates the discord slice.
///
/// `state` is the shared application state, handed to the bridge actor
/// when it spawns (it reads session context to shape gateway events).
///
/// # Panics
///
/// Panics if the connection slot is already registered — a double
/// activation is a wiring bug that must abort launch, not continue
/// degraded.
pub async fn activate(services: &mut Services, state: State) {
    // Mint the connection cell: the status actor is its single writer;
    // feature gates (to-thread) and the dashboard read it.
    #[expect(
        clippy::expect_used,
        reason = "double activation is a wiring bug; the slice must activate exactly once"
    )]
    let connection_cell = services
        .slices
        .register(
            super::discord_connection_slot(),
            ConnectionState {
                connected: false,
                detail: None,
            },
        )
        .expect("discord connection slot is registered exactly once at wiring");

    // Mint all three channels up front. A disabled bridge leaves the
    // bridge/gateway channels empty forever; nobody sends, so an empty
    // channel is free.
    let MintedChannels {
        parked,
        bridge_tx,
        gateway_tx,
        status_rx,
    } = DiscordGatewayChannels::mint();

    // Spawn the status actor (always): the connection authority. It
    // must be draining before any gateway status lands, so it comes up
    // before the bridge and long before the gateway task.
    let discord_status = DiscordStatusActor::supervise(
        &services.root_supervisor,
        DiscordStatusActorDeps {
            deps: ActorDeps {
                services: services.clone(),
            },
            status_rx: status_rx.to_async(),
            cell: connection_cell,
        },
    )
    .restart_policy(kameo::supervision::RestartPolicy::Never)
    .spawn()
    .await;
    discord_status.wait_for_startup().await;

    // Conditionally spawn the bridge actor: bus events → the gateway
    // channels. The gateway task itself is spawned later by the
    // frontend (`jinn_discord::spawn_gateway`), so it never blocks
    // readiness. When disabled, the senders drop here — the parked
    // receivers then report `Disconnected`, fail-closed.
    if services.user_preferences_storage.read().discord.enabled {
        let _discord_bridge = super::DiscordBridgeActor::supervise(
            &services.root_supervisor,
            super::DiscordBridgeActorDeps {
                deps: ActorDeps {
                    services: services.clone(),
                },
                tx: bridge_tx,
                gateway_tx,
                state,
                session_cap: crate::common::tcaps::mint::mint_session_cap(),
            },
        )
        .restart_policy(kameo::supervision::RestartPolicy::Never)
        .spawn()
        .await;
    }

    // Park the gateway-facing halves on Services. The frontend pulls
    // them at spawn time; until then they sit idle.
    services.discord = parked;

    // Route rows: the `gdc` sequence in the Normal scope.
    super::attach_discord_rows(&services.key_routes);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, reason = "test code")]
    use super::*;
    use crate::feat::discord::DiscordConfig;

    /// Sets `[discord] enabled` on the services' preference storage.
    fn set_enabled(services: &Services, enabled: bool) {
        let mut prefs = services.user_preferences_storage.read();
        prefs.discord = DiscordConfig {
            enabled,
            ..DiscordConfig::default()
        };
        services
            .user_preferences_storage
            .save(&prefs)
            .expect("save prefs");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn activate_with_disabled_bridge_parks_channels_and_attaches_rows() {
        // Given fake services with discord disabled.
        let mut services = crate::Services::new_fake().await;
        set_enabled(&services, false);
        let state = State::new(crate::common::app_state::AppState::default());

        // When activating the slice.
        activate(&mut services, state).await;

        // Then the connection cell is minted and readable.
        assert!(
            services
                .slices
                .reader::<ConnectionState>(&super::super::discord_connection_slot())
                .is_some(),
            "connection cell must be minted"
        );
        // And the to-thread route row is attached.
        let rows = services.key_routes.rows();
        assert!(
            rows.iter()
                .any(|row| row.route_id == super::super::key_routes::route_ids::TO_THREAD),
            "to-thread row must be attached"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn activate_with_disabled_bridge_leaves_parked_receivers_disconnected() {
        // Given fake services with discord disabled and an activation.
        let mut services = crate::Services::new_fake().await;
        set_enabled(&services, false);
        let state = State::new(crate::common::app_state::AppState::default());
        activate(&mut services, state).await;

        // When the parked bridge channel is drained.
        // Then it reports disconnect — the bridge never spawned, so its
        // sender dropped at the disabled branch (fail-closed, no hang).
        let err = services
            .discord
            .bridge_rx
            .clone()
            .try_recv()
            .expect_err("no message was ever sent");
        assert!(matches!(err, kanal::ReceiveError::SendClosed));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn double_activation_panics_on_the_connection_slot() {
        // Given services already activated.
        let mut services = crate::Services::new_fake().await;
        let state = State::new(crate::common::app_state::AppState::default());
        activate(&mut services, state).await;

        // When activating a second time.
        // Then the wiring bug aborts: the connection slot is already taken.
        let result = tokio::spawn(async move {
            activate(
                &mut services,
                State::new(crate::common::app_state::AppState::default()),
            )
            .await;
        })
        .await;
        assert!(
            result.is_err(),
            "double activation must panic on the taken slot"
        );
    }
}
