//! Discord's parked gateway channels.
//!
//! Discord's three kanal channels are created unconditionally at
//! [`activate`](super::activate) — a disabled bridge never fills them; an
//! empty channel is free. The receiving halves (plus the status channel's
//! sender, which the gateway feeds) are returned by `activate()` so the
//! gateway frontend (`jinn-discord`) can pull them at spawn time without
//! the kernel threading discord types through its signatures.

/// The channel halves the Discord gateway consumes.
///
/// Created by discord's `activate()`; parked on `Services` until
/// `jinn_discord::spawn_gateway` drains them. Capacities are the
/// historical ones (bridge 64, gateway requests 16, status unbounded).
///
/// The status channel's *receiver* is not parked — the status actor is
/// spawned by `activate()` itself and takes it directly.
#[derive(Debug, Clone)]
pub struct DiscordGatewayChannels {
    /// Bridge events (bus → gateway), bounded 64. Pairs with the
    /// sender held by the bridge actor.
    pub bridge_rx: kanal::AsyncReceiver<jinn_discord_msg::BridgeEvent>,
    /// Gateway requests (bridge → gateway), bounded 16. Pairs with the
    /// sender held by the bridge actor.
    pub gateway_rx: kanal::AsyncReceiver<jinn_discord_msg::GatewayRequest>,
    /// Status updates (gateway → status actor), unbounded. The gateway
    /// holds this sender half.
    pub status_tx: kanal::Sender<jinn_discord_msg::DiscordStatusUpdate>,
}

/// A freshly-minted channel set plus the partner halves that stay
/// behind at the mint site.
pub struct MintedChannels {
    /// The halves handed to the gateway frontend at spawn time.
    pub parked: DiscordGatewayChannels,
    /// Bridge-event sender, handed to the bridge actor when it spawns.
    pub bridge_tx: kanal::Sender<jinn_discord_msg::BridgeEvent>,
    /// Gateway-request sender, handed to the bridge actor when it spawns.
    pub gateway_tx: kanal::Sender<jinn_discord_msg::GatewayRequest>,
    /// Status-update receiver, handed to the status actor.
    pub status_rx: kanal::Receiver<jinn_discord_msg::DiscordStatusUpdate>,
}

impl DiscordGatewayChannels {
    /// Mints a live channel set with the historical capacities.
    #[must_use]
    pub fn mint() -> MintedChannels {
        let (bridge_tx, bridge_rx) = kanal::bounded::<jinn_discord_msg::BridgeEvent>(64);
        let (gateway_tx, gateway_rx) = kanal::bounded::<jinn_discord_msg::GatewayRequest>(16);
        let (status_tx, status_rx) = kanal::unbounded::<jinn_discord_msg::DiscordStatusUpdate>();
        MintedChannels {
            parked: Self {
                bridge_rx: bridge_rx.to_async(),
                gateway_rx: gateway_rx.to_async(),
                status_tx,
            },
            bridge_tx,
            gateway_tx,
            status_rx,
        }
    }

    /// A dummy set: live channels whose partner halves are dropped
    /// immediately. Receives report `Disconnected` and sends report
    /// `SendError` — both fail closed. Used for the pre-activate
    /// default on `Services`, where nothing sends (the gateway is not
    /// spawned) and a disabled config never drains.
    #[must_use]
    pub fn detached() -> Self {
        let MintedChannels {
            parked,
            bridge_tx,
            gateway_tx,
            status_rx,
        } = Self::mint();
        drop(bridge_tx);
        drop(gateway_tx);
        drop(status_rx);
        parked
    }
}
