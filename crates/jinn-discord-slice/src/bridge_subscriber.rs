//! The Discord bridge subscriber — session events to gateway channels,
//! on trouper.
//!
//! A [`ServiceActor`] subscribed to the `jinn.session` topic (fed by
//! the core bridge's forward routes). It replaces the former kameo
//! bridge actor: the crossing is now bus → relay → topic (kernel
//! wiring) + this subscriber (slice folding), and no kameo actor lives
//! in the slice.
//!
//! # What it folds
//!
//! - [`SessionPhaseChanged`] with `new_phase == Idle` →
//!   [`BridgeEvent::TurnFinished`]
//! - [`SessionSetupCompleted`] → [`BridgeEvent::SetupCompleted`]
//! - [`SessionTeardownFinished`] → [`BridgeEvent::TeardownFinished`]
//! - [`SessionArchived`] → [`BridgeEvent::Archived`]
//! - [`CreateThreadForSession`] → [`GatewayRequest::CreateThreadForSession`]
//!   on the gateway-request channel
//! - [`DiscordThreadCreated`] / [`DiscordThreadCreateFailed`] → a
//!   [`ChatEntry`] pushed directly into the session's history
//!
//! All other topic traffic is ignored. The bot never sees streaming
//! tokens or intermediate tool calls — it only acts on turn boundaries
//! and lifecycle results.

use jinn_core_types::SessionId;
use jinn_domain::common::state::State;
use jinn_domain::feat::session::chat_entry::ChatEntry;
use jinn_discord_msg::{
    BridgeEvent, CreateThreadForSession, CreateThreadReason, DiscordThreadCreateFailed,
    DiscordThreadCreated, ForumChannelError, GatewayRequest,
};
use jinn_session_msg::{
    SessionArchived, SessionPhaseChanged, SessionSetupCompleted, SessionTeardownFinished,
};
use trouper::actor::{MsgHandler, ServiceActor};
use trouper::context::MsgCtx;
use trouper::registry::RegistryError;
use trouper::system::ActorSystem;
use trouper::types::ActorPath;

/// The Discord bridge subscriber.
///
/// Holds the sender halves of the two gateway kanal channels, a clone
/// of [`State`], and the session capability — the fold writes the
/// `gdc` (to-thread) result `ChatEntry` inline on outcome events.
pub struct DiscordBridgeSubscriber {
    /// Forwards topic events onto this channel as [`BridgeEvent`]s.
    tx: kanal::Sender<BridgeEvent>,
    /// Forwards `CreateThreadForSession` requests onto this channel as
    /// [`GatewayRequest`]s — the reverse direction (domain → gateway
    /// do-something).
    gateway_tx: kanal::Sender<GatewayRequest>,
    /// Shared application state — writes the `gdc` (to-thread) result
    /// `ChatEntry` back into the targeted session's history.
    state: State,
    /// Authority to push entries into sessions.
    session_cap: jinn_domain::common::tcaps::session::SessionCap,
}

/// Dependencies for [`DiscordBridgeSubscriber`].
#[derive(Clone)]
pub struct DiscordBridgeSubscriberDeps {
    /// Sender half of the bounded (64) bridge channel.
    pub tx: kanal::Sender<BridgeEvent>,
    /// Sender half of the bounded (16) gateway-request channel.
    pub gateway_tx: kanal::Sender<GatewayRequest>,
    /// Shared application state.
    pub state: State,
    /// Authority to push entries into sessions.
    pub session_cap: jinn_domain::common::tcaps::session::SessionCap,
}

impl DiscordBridgeSubscriber {
    /// Spawns the subscriber at `discord-bridge` and subscribes it to
    /// the `jinn.session` topic.
    ///
    /// A successful [`ActorSystem::subscribe`] is the ordering
    /// guarantee: the topic cursor is registered before any gateway
    /// traffic can flow (the channels are parked until the frontend
    /// spawns the gateway), so no crossing event is missed.
    ///
    /// # Panics
    ///
    /// Panics if the topic subscription fails, which can only happen
    /// on a broken actor system; the slice activation ordering relies
    /// on the cursor being registered.
    pub fn spawn(system: &std::sync::Arc<ActorSystem>, deps: DiscordBridgeSubscriberDeps) -> ActorPath {
        let DiscordBridgeSubscriberDeps {
            tx,
            gateway_tx,
            state,
            session_cap,
        } = deps;
        let path = trouper::builder::spawn_service_builder::<Self>(system)
            .at(ActorPath::new("discord-bridge"))
            .start_with({
                move || {
                    Box::pin(async move {
                        Ok(Self {
                            tx,
                            gateway_tx,
                            state,
                            session_cap,
                        })
                    })
                }
            })
            .handles::<SessionPhaseChanged>()
            .handles::<SessionSetupCompleted>()
            .handles::<SessionTeardownFinished>()
            .handles::<SessionArchived>()
            .handles::<CreateThreadForSession>()
            .handles::<DiscordThreadCreated>()
            .handles::<DiscordThreadCreateFailed>()
            .start();

        #[expect(
            clippy::expect_used,
            reason = "subscription failure is a broken actor system, not a caller bug;                       the channel-parked-before-gateway ordering relies on the cursor"
        )]
        system
            .subscribe(&path, &jinn_session_msg::session_topic(), None)
            .expect("discord bridge subscriber subscribes to the session topic");
        path
    }
}

impl ServiceActor for DiscordBridgeSubscriber {
    async fn start(
        _args: &serde_json::Value,
    ) -> Result<Self, trouper::error_stack::Report<RegistryError>> {
        // Never called: the spawn helper injects the channels, state,
        // and capability via `start_with`.
        Err(trouper::error_stack::IntoReport::into_report(
            RegistryError::InvalidSpec,
        )
        .attach("DiscordBridgeSubscriber is spawned via start_with"))
    }
}

impl MsgHandler<SessionPhaseChanged> for DiscordBridgeSubscriber {
    async fn handle(&mut self, msg: SessionPhaseChanged, _ctx: &mut MsgCtx<'_>) {
        self.handle_session_phase_changed(&msg);
    }
}

impl MsgHandler<SessionSetupCompleted> for DiscordBridgeSubscriber {
    async fn handle(&mut self, msg: SessionSetupCompleted, _ctx: &mut MsgCtx<'_>) {
        self.handle_session_setup_completed(&msg);
    }
}

impl MsgHandler<SessionTeardownFinished> for DiscordBridgeSubscriber {
    async fn handle(&mut self, msg: SessionTeardownFinished, _ctx: &mut MsgCtx<'_>) {
        self.handle_session_teardown_finished(&msg);
    }
}

impl MsgHandler<SessionArchived> for DiscordBridgeSubscriber {
    async fn handle(&mut self, msg: SessionArchived, _ctx: &mut MsgCtx<'_>) {
        self.handle_session_archived(&msg);
    }
}

impl MsgHandler<CreateThreadForSession> for DiscordBridgeSubscriber {
    async fn handle(&mut self, msg: CreateThreadForSession, _ctx: &mut MsgCtx<'_>) {
        self.forward_gateway_request(GatewayRequest::CreateThreadForSession {
            session_id: msg.session_id,
            title: msg.title,
        });
    }
}

impl MsgHandler<DiscordThreadCreated> for DiscordBridgeSubscriber {
    async fn handle(&mut self, msg: DiscordThreadCreated, _ctx: &mut MsgCtx<'_>) {
        self.handle_created(&msg);
    }
}

impl MsgHandler<DiscordThreadCreateFailed> for DiscordBridgeSubscriber {
    async fn handle(&mut self, msg: DiscordThreadCreateFailed, _ctx: &mut MsgCtx<'_>) {
        self.handle_failed(&msg);
    }
}

impl DiscordBridgeSubscriber {
    /// Constructs a subscriber instance directly (for tests that call
    /// the fold helpers; added in the Phase 4 test port).
    #[cfg(test)]
    pub(crate) fn new(
        tx: kanal::Sender<BridgeEvent>,
        state: State,
        session_cap: jinn_domain::common::tcaps::session::SessionCap,
    ) -> Self {
        let (gateway_tx, _gateway_rx) = kanal::bounded(1);
        Self {
            tx,
            gateway_tx,
            state,
            session_cap,
        }
    }
}

impl DiscordBridgeSubscriber {
    /// Forward phase changes to the gateway **only** when the new phase is
    /// `Idle`. Non-idle transitions (Streaming, Sending, …) are dropped.
    pub(super) fn handle_session_phase_changed(&self, payload: &SessionPhaseChanged) {
        if payload.new_phase != jinn_session_msg::PhaseKind::Idle {
            return;
        }
        self.forward(BridgeEvent::TurnFinished {
            session_id: payload.session_id.clone(),
        });
    }

    /// Forward every setup completion (success or failure — the gateway
    /// formats the message from `cwd`/`error`).
    pub(super) fn handle_session_setup_completed(&self, payload: &SessionSetupCompleted) {
        self.forward(BridgeEvent::SetupCompleted {
            session_id: payload.session_id.clone(),
            cwd: payload.cwd.clone(),
            error: payload.error.clone(),
        });
    }

    /// Forward every teardown completion (success or failure — the gateway
    /// formats the message from `error`).
    pub(super) fn handle_session_teardown_finished(&self, payload: &SessionTeardownFinished) {
        self.forward(BridgeEvent::TeardownFinished {
            session_id: payload.session_id.clone(),
            error: payload.error.clone(),
        });
    }

    /// Forward every archive completion to the gateway.
    pub(super) fn handle_session_archived(&self, payload: &SessionArchived) {
        self.forward(BridgeEvent::Archived {
            session_id: payload.session_id.clone(),
        });
    }

    // ── to-thread feedback (reverse: gateway → jinn session history) ─────

    /// Handle `DiscordThreadCreated`: push a system `ChatEntry` mentioning the title.
    pub(super) fn handle_created(&self, msg: &DiscordThreadCreated) {
        let entry = ChatEntry::system(format!("Continuing in Discord thread: {}", msg.title));
        push_entry(&self.state, self.session_cap.clone(), &msg.session_id, entry);
    }

    /// Handle `DiscordThreadCreateFailed`: push an error `ChatEntry`.
    pub(super) fn handle_failed(&self, msg: &DiscordThreadCreateFailed) {
        let entry = ChatEntry::error(reason_message(&msg.reason));
        push_entry(&self.state, self.session_cap.clone(), &msg.session_id, entry);
    }

    /// Push one event onto the channel.
    ///
    /// A full channel means the gateway task is behind; rather than block the
    /// topic dispatch we drop with a warning. The next `Idle`/setup event
    /// will still arrive and trigger a fresh read from `State`.
    fn forward(&self, event: BridgeEvent) {
        tracing::info!(event = %event_discriminant(&event), "discord bridge forwarding");
        if !matches!(self.tx.try_send(event), Ok(true)) {
            tracing::warn!("discord bridge channel full — event dropped");
        }
    }

    /// Push one gateway request onto the request channel.
    ///
    /// Same drop-on-full semantics as [`forward`](Self::forward) — a full
    /// channel means the gateway task is behind, so we drop with a warning
    /// rather than block the topic dispatch.
    fn forward_gateway_request(&self, request: GatewayRequest) {
        tracing::info!("discord bridge forwarding gateway request");
        if !matches!(self.gateway_tx.try_send(request), Ok(true)) {
            tracing::warn!("discord gateway request channel full — request dropped");
        }
    }
}

/// Short label identifying a [`BridgeEvent`] variant for log lines.
///
/// The events themselves may carry large payloads (session ids are fine,
/// but keeping a single helper avoids per-arm `Display` requirements).
fn event_discriminant(event: &BridgeEvent) -> &'static str {
    match event {
        BridgeEvent::SetupCompleted { .. } => "SetupCompleted",
        BridgeEvent::TurnFinished { .. } => "TurnFinished",
        BridgeEvent::TeardownFinished { .. } => "TeardownFinished",
        BridgeEvent::Archived { .. } => "Archived",
    }
}

/// Push a `ChatEntry` into a session by id; drop silently if the session is
/// gone (closed/archived concurrently since the `gdc` request was emitted).
fn push_entry(
    state: &State,
    session_cap: jinn_domain::common::tcaps::session::SessionCap,
    session_id: &SessionId,
    entry: ChatEntry,
) {
    state.with_session(&session_cap, |view| {
        if let Some(session) = view.session.map().get_mut(session_id) {
            session.push_entry(entry);
        } else {
            tracing::debug!(
                %session_id,
                "to-thread result arrived for a session that no longer exists; dropping",
            );
        }
    });
}

/// Render a human-readable message for each failure reason.
fn reason_message(reason: &CreateThreadReason) -> String {
    match reason {
        CreateThreadReason::AlreadyBound => concat!(
            "Can't continue in Discord: this session is already in a Discord ",
            "thread — continue there."
        )
        .to_owned(),
        CreateThreadReason::ForumChannel(ForumChannelError::Missing) => concat!(
            "Can't continue in Discord: no `forum_channel` is set in ",
            "`[discord]`. Set it to the numeric channel id (snowflake) ",
            "of a `GUILD_FORUM` channel the bot can manage."
        )
        .to_owned(),
        CreateThreadReason::ForumChannel(ForumChannelError::Invalid { value }) => {
            format!(
                "Can't continue in Discord: `forum_channel` must be a numeric channel id (snowflake), but it's set to `{value}`. Copy the channel id in Discord (right-click → Copy Channel ID) and paste it into `[discord] forum_channel`."
            )
        }
        CreateThreadReason::CreateFailed(detail) => {
            format!("Couldn't create the Discord thread: {detail}")
        }
        CreateThreadReason::MappingWriteFailed => concat!(
            "Discord thread was created, but jinn couldn't record the binding — ",
            "the thread exists but won't receive replies. See the logs."
        )
        .to_owned(),
    }
}
