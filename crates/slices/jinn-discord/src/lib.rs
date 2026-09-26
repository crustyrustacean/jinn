//! The Discord slice — drive a running jinn from Discord.
//!
//! Owns the connection authority (status actor on trouper), the bridge
//! actor (bus → gateway channels), the thread-map DAO, config, the
//! message splitter, final-reply extraction, route rows, the
//! `[discord]` config section, and the poise gateway itself (Discord
//! websocket + slash commands, under [`backend`], re-exported at the
//! crate root).

pub mod authorize;
pub mod backend;
pub mod bridge_subscriber;
pub mod channels;
pub mod config;
pub mod key_routes;
pub mod message_split;
pub mod reply;
pub mod route;
pub mod status_actor;
pub mod thread_map;
pub mod to_thread_intent;

pub use backend::spawn_gateway;
pub use bridge_subscriber::DiscordBridgeSubscriber;
pub use bridge_subscriber::DiscordBridgeSubscriberDeps;
pub use channels::DiscordGatewayChannels;
pub use channels::MintedChannels;
pub use config::DiscordConfig;
pub use jinn_discord_msg::BridgeEvent;
pub use jinn_discord_msg::CreateThreadForSession;
pub use jinn_discord_msg::CreateThreadReason;
pub use jinn_discord_msg::DiscordStatusUpdate;
pub use jinn_discord_msg::DiscordThreadCreateFailed;
pub use jinn_discord_msg::DiscordThreadCreated;
pub use jinn_discord_msg::ForumChannelError;
pub use jinn_discord_msg::GatewayRequest;
pub use jinn_discord_msg::ThreadId;
pub use key_routes::attach_discord_rows;
pub use key_routes::discord_scope;
pub use message_split::split_message;
pub use reply::FinalReply;
pub use reply::read_final_reply;
pub use route::RouteDecision;
pub use route::route_decision;
pub use status_actor::ConnectionState;
pub use status_actor::DiscordStatusActor;
pub use status_actor::DiscordStatusActorDeps;
pub use status_actor::discord_connection_slot;
pub use thread_map::DiscordThreadMap;
pub use thread_map::DiscordThreadMapError;
pub use thread_map::ThreadMapping;

/// The parked gateway-facing channels + config an activation hands to
/// the frontend (`spawn_gateway`, [`crate::backend`]).
#[derive(Debug)]
pub struct ActivatedDiscord {
    /// The gateway's receiving halves (bridge events, gateway
    /// requests) + the status sender.
    pub parked: DiscordGatewayChannels,
    /// The validated `[discord]` section (activation resolved it).
    pub config: DiscordConfig,
}

/// Activates the discord slice through the host verbs.
///
/// Mint the connection cell, spawn the status actor on trouper (always
/// — it is the connection authority regardless of bridge enablement),
/// read + validate the `[discord]` section (fail-fast when present),
/// set the slice's feature flag from it, create all three gateway kanal
/// channels unconditionally, spawn the bridge subscriber only when
/// enabled, and attach the route rows. Returns the gateway-facing
/// halves + the config for the frontend spawn.
///
/// The `resolve` closure is the kernel's document lookup; it is
/// applied to the staged sections **inside** activation so the
/// `[discord]` value exists before the slice reads it — sections that
/// resolve only at composition's `finalize` used to hand the slice
/// `T::default()` and read as a silently disabled bridge.
///
/// The section is optional: a document without `[discord]` activates
/// the slice disabled, which is what a fresh install looks like. A
/// section that is present but malformed still aborts.
///
/// # Errors
///
/// Returns [`SliceConfigError`] when the `[discord]` section is
/// present but malformed — activation is the fail-fast gate.
pub async fn activate(
    host: &mut jinn_slices::AppSliceHost<'_>,
    services: &jinn_domain::Services,
    state: jinn_domain::common::state::State,
) -> Result<ActivatedDiscord, jinn_config::ConfigSectionError> {
    // Config: read through the layer at the point of use. A stock
    // `jinn.toml` carries no `[discord]` table, so absence must be a
    // normal configuration (the disabled default) rather than a launch
    // abort; a present-but-malformed one still aborts here.
    let config = services.config.get::<DiscordConfig>()?;

    let system = host.system().clone();

    // Mint all three channels up front. A disabled bridge leaves the
    // bridge/gateway channels empty forever; nobody sends, so an empty
    // channel is free.
    let MintedChannels {
        parked,
        bridge_tx,
        gateway_tx,
        status_rx,
    } = DiscordGatewayChannels::mint();

    // Status actor: the connection authority, on trouper. Must be
    // draining before any gateway status lands.
    status_actor::DiscordStatusActor::spawn(DiscordStatusActorDeps {
        status_rx: status_rx.to_async(),
        cell: host
            .register_cell(
                discord_connection_slot(),
                ConnectionState {
                    connected: false,
                    detail: None,
                },
            )
            .expect("discord connection slot is registered exactly once at wiring"),
        bus: services.bus.clone(),
        system,
    });

    host.set_flag("discord", config.enabled);

    // Conditionally spawn the bridge subscriber: trouper `jinn.session`
    // topic events → the gateway channels. The gateway task itself is
    // spawned later by the frontend (`spawn_gateway`), so
    // it never blocks readiness. When disabled, the senders drop here —
    // the parked receivers then report `Disconnected`, fail-closed.
    if config.enabled {
        bridge_subscriber::DiscordBridgeSubscriber::spawn(
            &services.trouper_system,
            bridge_subscriber::DiscordBridgeSubscriberDeps {
                tx: bridge_tx,
                gateway_tx,
                state,
            },
        );
    }

    // Route rows: the `gdc` sequence in the Normal scope.
    attach_discord_rows(host.key_routes());

    Ok(ActivatedDiscord { parked, config })
}

#[cfg(test)]
mod activate_tests {
    #![allow(clippy::expect_used, clippy::panic, reason = "test code")]

    use super::activate;
    use jinn_slices::KeyRoutes;
    use jinn_slices::OverlayViews;
    use jinn_slices::Slices;
    use jinn_slices::host::SliceHost;
    use jinn_slices::view::Viewport;

    /// Fake services whose configuration layer carries a `[discord]`
    /// section with `body` as its contents.
    async fn services_with_discord(body: &str) -> jinn_domain::Services {
        let mut services = jinn_domain::Services::new_fake().await;
        services.config = jinn_config::testutil::config_layer(&format!("[discord]\n{body}"));
        services
    }

    /// Fake services over a document that carries no `[discord]` section —
    /// a stock install.
    async fn services_without_discord() -> jinn_domain::Services {
        let mut services = jinn_domain::Services::new_fake().await;
        services.config = jinn_config::testutil::config_layer("");
        services
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn activation_resolves_the_config_section_before_reading_it() {
        // Given a fresh slice host and a document sink whose [discord]
        // section enables the bridge.
        let slices = Slices::new();
        let key_routes = KeyRoutes::new();
        let mut viewport = Viewport::new();
        let overlay_views = OverlayViews::<jinn_slices::RenderFacts>::new();
        let services = services_with_discord("enabled = true").await;
        let state = jinn_domain::common::state::State::new(
            jinn_domain::common::app_state::AppState::default(),
        );
        let mut host = SliceHost::new(
            &slices,
            &mut viewport,
            &overlay_views,
            &key_routes,
            &services.trouper_system,
        );

        // When activating the slice through the real path.
        let activated = activate(&mut host, &services, state)
            .await
            .expect("activation resolves the section");

        // Then the resolved config carries the document value, not defaults.
        assert!(activated.config.enabled, "config.enabled from the document");
        // And the slice's feature flag is set from it.
        assert!(slices.flag("discord"), "flag mirrors the resolved config");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn absent_discord_section_activates_disabled() {
        // Given a document with no [discord] section — a stock install.
        let slices = Slices::new();
        let key_routes = KeyRoutes::new();
        let mut viewport = Viewport::new();
        let overlay_views = OverlayViews::<jinn_slices::RenderFacts>::new();
        let services = services_without_discord().await;
        let state = jinn_domain::common::state::State::new(
            jinn_domain::common::app_state::AppState::default(),
        );
        let mut host = SliceHost::new(
            &slices,
            &mut viewport,
            &overlay_views,
            &key_routes,
            &services.trouper_system,
        );

        // When activating the slice through the real path.
        let activated = activate(&mut host, &services, state)
            .await
            .expect("an absent section is a normal configuration");

        // Then the slice activates disabled rather than aborting the launch.
        assert!(!activated.config.enabled, "absence reads the default");
        // And the feature flag mirrors it.
        assert!(!slices.flag("discord"), "flag mirrors the default");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn activation_fails_fast_on_a_malformed_section() {
        // Given a document sink whose [discord] table is malformed.
        let slices = Slices::new();
        let key_routes = KeyRoutes::new();
        let mut viewport = Viewport::new();
        let overlay_views = OverlayViews::<jinn_slices::RenderFacts>::new();
        let services = services_with_discord("enabled = \"maybe\"").await;
        let state = jinn_domain::common::state::State::new(
            jinn_domain::common::app_state::AppState::default(),
        );
        let mut host = SliceHost::new(
            &slices,
            &mut viewport,
            &overlay_views,
            &key_routes,
            &services.trouper_system,
        );

        // When activating.
        let result = activate(&mut host, &services, state).await;

        // Then activation is the fail-fast gate: a section that is
        // present but malformed errors instead of proceeding on
        // defaults. Optional means absence is legal, not that a typo
        // silently disables the bridge.
        assert!(result.is_err(), "malformed section aborts activation");
    }
}
