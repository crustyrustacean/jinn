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
pub use jinn_discord_msg::discord_topic;
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
/// read + validate the `[discord]` section (fail-fast), set the
/// slice's feature flag from it, create all three gateway kanal
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
/// # Errors
///
/// Returns [`SliceConfigError`] when the `[discord]` section is
/// malformed — activation is the fail-fast gate.
pub async fn activate(
    host: &mut jinn_slices::AppSliceHost<'_>,
    services: &jinn_domain::Services,
    state: jinn_domain::common::state::State,
    resolve: &dyn Fn(&str) -> Option<toml::Table>,
) -> Result<ActivatedDiscord, jinn_slices::ConfigSectionError> {
    // Config section: typed read through the host, resolved eagerly —
    // the value must exist before `take()` below.
    let config_handle = host.config_section::<DiscordConfig>("discord");
    host.apply_sections(resolve)?;

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

    let config = config_handle.take();
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
                session_cap: jinn_domain::common::tcaps::mint::mint_session_cap(),
            },
        );
    }

    // Forward routes: the session-family messages this slice consumes
    // cross the core bridge onto `jinn.session`. Composition drains the
    // staged set into per-route relays; the relay is just one more bus
    // subscriber, so existing kameo consumers are unaffected.
    let topic = jinn_session_msg::session_topic();
    host.forward::<jinn_session_msg::SessionPhaseChanged, _>(topic.clone(), || {
        <jinn_session_msg::SessionPhaseChanged as trouper::schema::Schema>::schema_def()
    });
    host.forward::<jinn_session_msg::SessionSetupCompleted, _>(topic.clone(), || {
        <jinn_session_msg::SessionSetupCompleted as trouper::schema::Schema>::schema_def()
    });
    host.forward::<jinn_session_msg::SessionTeardownFinished, _>(topic.clone(), || {
        <jinn_session_msg::SessionTeardownFinished as trouper::schema::Schema>::schema_def()
    });
    host.forward::<jinn_session_msg::SessionArchived, _>(topic.clone(), || {
        <jinn_session_msg::SessionArchived as trouper::schema::Schema>::schema_def()
    });
    host.forward::<CreateThreadForSession, _>(topic.clone(), || {
        <CreateThreadForSession as trouper::schema::Schema>::schema_def()
    });
    host.forward::<DiscordThreadCreated, _>(topic.clone(), || {
        <DiscordThreadCreated as trouper::schema::Schema>::schema_def()
    });
    host.forward::<DiscordThreadCreateFailed, _>(topic, || {
        <DiscordThreadCreateFailed as trouper::schema::Schema>::schema_def()
    });

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

    /// A stub document sink carrying a `[discord]` section body.
    fn stub_doc(body: &'static str) -> impl Fn(&str) -> Option<toml::Table> {
        let table: toml::Table = toml::from_str(body).expect("stub TOML parses");
        let sections = std::collections::HashMap::from([("discord".to_owned(), table)]);
        move |key| sections.get(key).cloned()
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
        let services = jinn_domain::Services::new_fake().await;
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
        let resolve = stub_doc("enabled = true");

        // When activating the slice through the real path.
        let activated = activate(&mut host, &services, state, &resolve)
            .await
            .expect("activation resolves the section");

        // Then the resolved config carries the document value, not defaults.
        assert!(activated.config.enabled, "config.enabled from the document");
        // And the slice's feature flag is set from it.
        assert!(slices.flag("discord"), "flag mirrors the resolved config");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn activation_fails_fast_on_a_malformed_section() {
        // Given a document sink whose [discord] table is malformed.
        let slices = Slices::new();
        let key_routes = KeyRoutes::new();
        let mut viewport = Viewport::new();
        let overlay_views = OverlayViews::<jinn_slices::RenderFacts>::new();
        let services = jinn_domain::Services::new_fake().await;
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
        let resolve = stub_doc("enabled = \"maybe\"");

        // When activating.
        let result = activate(&mut host, &services, state, &resolve).await;

        // Then activation is the fail-fast gate: it errors instead of
        // proceeding on defaults.
        assert!(result.is_err(), "malformed section aborts activation");
    }
}
