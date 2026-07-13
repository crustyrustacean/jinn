//! The Discord bridge actor — a thin kameo actor that subscribes to jinn's bus
//! and forwards the events the bot cares about onto a bounded channel.
//!
//! # Why this exists
//!
//! Bus subscription requires a kameo actor handle (the bus hands events to an
//! `ActorRef`'s recipient). The poise gateway, by contrast, runs as a plain
//! tokio task owning its own websocket. This actor is the bridge between those
//! two worlds: it is the *only* thing that subscribes, and it just forwards.
//!
//! # What it forwards
//!
//! - [`SessionPhaseChanged`] with `new_phase == Idle` → [`BridgeEvent::TurnFinished`]
//! - [`SessionSetupCompleted`] → [`BridgeEvent::SetupCompleted`]
//!
//! All other bus traffic is ignored. The bot never sees streaming tokens or
//! intermediate tool calls — it only acts on turn boundaries and setup results.

use kameo::prelude::{Actor, ActorRef, Context, Message};

use crate::SessionId;
use crate::common::actor_deps::ActorDeps;
use crate::common::state::State;
use crate::feat::session::chat_entry::ChatEntry;
use crate::feat::session::phase_machine::PhaseKind;
use crate::feat::session::protocol::session_archived::SessionArchived;
use crate::feat::session::protocol::session_phase_changed::SessionPhaseChanged;
use crate::feat::session_lifecycle::protocol::event::{
    SessionSetupCompleted, SessionTeardownFinished,
};

use super::protocol::{
    BridgeEvent, CreateThreadForSession, CreateThreadReason, DiscordThreadCreateFailed,
    DiscordThreadCreated, ForumChannelError, GatewayRequest,
};

/// The bridge actor.
///
/// Holds a clone of the bus subscription handle (`ActorDeps`), the sender
/// half of the channel the gateway task drains, and a clone of [`State`] for
/// writing the `gdc` (to-thread) result `ChatEntry` inline on result events.
pub struct DiscordBridgeActor {
    /// Forwards bus events onto this channel as [`BridgeEvent`]s.
    tx: kanal::Sender<BridgeEvent>,
    /// Forwards `CreateThreadForSession` bus commands onto this channel as
    /// [`GatewayRequest`]s — the reverse direction (domain → gateway do-something).
    gateway_tx: kanal::Sender<GatewayRequest>,
    /// Shared application state — writes the `gdc` (to-thread) result
    /// `ChatEntry` back into the targeted session's history.
    state: State,
}

/// Dependencies for [`DiscordBridgeActor`].
#[derive(Clone)]
pub struct DiscordBridgeActorDeps {
    /// Universal actor dependencies (bus, services, etc.).
    pub deps: ActorDeps,
    /// Sender half of the bounded (64) bridge channel.
    pub tx: kanal::Sender<BridgeEvent>,
    /// Sender half of the bounded (16) gateway-request channel.
    pub gateway_tx: kanal::Sender<GatewayRequest>,
    /// Shared application state.
    pub state: State,
}

impl Actor for DiscordBridgeActor {
    type Args = DiscordBridgeActorDeps;
    type Error = std::convert::Infallible;

    async fn on_start(args: Self::Args, actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        args.deps
            .subscribe(actor_ref.clone().recipient::<SessionPhaseChanged>())
            .await;
        args.deps
            .subscribe(actor_ref.clone().recipient::<SessionSetupCompleted>())
            .await;
        args.deps
            .subscribe(actor_ref.clone().recipient::<SessionTeardownFinished>())
            .await;
        args.deps
            .subscribe(actor_ref.clone().recipient::<SessionArchived>())
            .await;
        args.deps
            .subscribe(actor_ref.clone().recipient::<CreateThreadForSession>())
            .await;
        // Feedback: write the gdc (to-thread) outcome back into the session's
        // chat history. These two events are published by the gateway after it
        // creates (or fails to create) a Discord forum thread.
        args.deps
            .subscribe(actor_ref.clone().recipient::<DiscordThreadCreated>())
            .await;
        args.deps
            .subscribe(actor_ref.recipient::<DiscordThreadCreateFailed>())
            .await;

        Ok(Self {
            tx: args.tx,
            gateway_tx: args.gateway_tx,
            state: args.state,
        })
    }
}

impl Message<SessionPhaseChanged> for DiscordBridgeActor {
    type Reply = ();

    async fn handle(&mut self, msg: SessionPhaseChanged, _ctx: &mut Context<Self, Self::Reply>) {
        self.handle_session_phase_changed(&msg);
    }
}

impl Message<SessionSetupCompleted> for DiscordBridgeActor {
    type Reply = ();

    async fn handle(&mut self, msg: SessionSetupCompleted, _ctx: &mut Context<Self, Self::Reply>) {
        self.handle_session_setup_completed(&msg);
    }
}

impl Message<SessionTeardownFinished> for DiscordBridgeActor {
    type Reply = ();

    async fn handle(
        &mut self,
        msg: SessionTeardownFinished,
        _ctx: &mut Context<Self, Self::Reply>,
    ) {
        self.handle_session_teardown_finished(&msg);
    }
}

impl Message<SessionArchived> for DiscordBridgeActor {
    type Reply = ();

    async fn handle(&mut self, msg: SessionArchived, _ctx: &mut Context<Self, Self::Reply>) {
        self.handle_session_archived(&msg);
    }
}

impl Message<CreateThreadForSession> for DiscordBridgeActor {
    type Reply = ();

    async fn handle(&mut self, msg: CreateThreadForSession, _ctx: &mut Context<Self, Self::Reply>) {
        self.forward_gateway_request(GatewayRequest::CreateThreadForSession {
            session_id: msg.session_id,
            title: msg.title,
        });
    }
}

impl Message<DiscordThreadCreated> for DiscordBridgeActor {
    type Reply = ();

    async fn handle(&mut self, msg: DiscordThreadCreated, _ctx: &mut Context<Self, Self::Reply>) {
        self.handle_created(&msg);
    }
}

impl Message<DiscordThreadCreateFailed> for DiscordBridgeActor {
    type Reply = ();

    async fn handle(
        &mut self,
        msg: DiscordThreadCreateFailed,
        _ctx: &mut Context<Self, Self::Reply>,
    ) {
        self.handle_failed(&msg);
    }
}

impl DiscordBridgeActor {
    /// Constructs an actor instance directly (for tests that bypass `on_start`).
    #[cfg(test)]
    pub(crate) fn new(tx: kanal::Sender<BridgeEvent>, state: State) -> Self {
        let (gateway_tx, _gateway_rx) = kanal::bounded(1);
        Self {
            tx,
            gateway_tx,
            state,
        }
    }

    /// Forward phase changes to the gateway **only** when the new phase is
    /// `Idle`. Non-idle transitions (Streaming, Sending, …) are dropped.
    pub(super) fn handle_session_phase_changed(&self, payload: &SessionPhaseChanged) {
        if payload.new_phase != PhaseKind::Idle {
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
        push_entry(&self.state, &msg.session_id, entry);
    }

    /// Handle `DiscordThreadCreateFailed`: push an error `ChatEntry`.
    pub(super) fn handle_failed(&self, msg: &DiscordThreadCreateFailed) {
        let entry = ChatEntry::error(reason_message(&msg.reason));
        push_entry(&self.state, &msg.session_id, entry);
    }

    /// Push one event onto the channel.
    ///
    /// A full channel means the gateway task is behind; rather than block the
    /// bus dispatch loop we drop with a warning. The next `Idle`/setup event
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
    /// rather than block the bus dispatch loop.
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
fn push_entry(state: &State, session_id: &SessionId, entry: ChatEntry) {
    let mut guard = state.write();
    match guard.try_session_mut(session_id) {
        Some(session) => {
            session.push_entry(entry);
        }
        None => {
            tracing::debug!(
                %session_id,
                "to-thread result arrived for a session that no longer exists; dropping",
            );
        }
    }
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

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, reason = "test code")]
    use super::*;
    use crate::common::app_state::AppState;
    use crate::feat::session::chat_entry::ChatEntryKind;
    use crate::protocol::SessionId;

    /// Build a bridge actor with one seeded session, plus its session id.
    ///
    /// The `tx`/`gateway_tx` channels are throwaways — these tests exercise
    /// the to-thread feedback handlers, not the forwarding path.
    fn actor_with_session() -> (DiscordBridgeActor, SessionId) {
        let (tx, _rx) = kanal::bounded(1);
        let state = State::new(AppState::default());
        let session_id = SessionId::new();
        // Seed the session so `try_session_mut` finds it.
        state.write().session_mut_or_create(&session_id);
        let actor = DiscordBridgeActor::new(tx, state);
        (actor, session_id)
    }

    /// `reason_message` for `AlreadyBound` mentions continuing in the existing thread.
    #[test]
    fn reason_message_already_bound_is_descriptive() {
        let msg = reason_message(&CreateThreadReason::AlreadyBound);
        assert!(msg.contains("already in a Discord thread"));
    }

    /// `reason_message` for `ForumChannel(Missing)` explains how to set the field.
    #[test]
    fn reason_message_forum_channel_missing_explains_how_to_set() {
        let msg = reason_message(&CreateThreadReason::ForumChannel(
            ForumChannelError::Missing,
        ));
        assert!(msg.contains("no `forum_channel` is set"));
        assert!(msg.contains("snowflake"));
        assert!(msg.contains("GUILD_FORUM"));
    }

    /// `reason_message` for `ForumChannel(Invalid)` shows the bad value and what
    /// a snowflake looks like.
    #[test]
    fn reason_message_forum_channel_invalid_shows_bad_value() {
        let msg = reason_message(&CreateThreadReason::ForumChannel(
            ForumChannelError::Invalid {
                value: "sessions".to_owned(),
            },
        ));
        assert!(
            msg.contains("`sessions`"),
            "expected the bad value in the message: {msg}"
        );
        assert!(msg.contains("snowflake"));
        assert!(msg.contains("Copy Channel ID"));
    }

    /// `reason_message` for `CreateFailed` includes the Discord error detail.
    #[test]
    fn reason_message_create_failed_includes_detail() {
        let msg = reason_message(&CreateThreadReason::CreateFailed("boom".to_owned()));
        assert!(msg.contains("boom"));
    }

    /// `reason_message` for `MappingWriteFailed` describes the orphaned-thread state.
    #[test]
    fn reason_message_mapping_write_failed_describes_orphan() {
        let msg = reason_message(&CreateThreadReason::MappingWriteFailed);
        assert!(msg.contains("won't receive replies"));
    }

    /// A `Created` event pushes a system entry mentioning the title.
    #[test]
    fn created_pushes_system_entry_with_title() {
        // Given an actor with one session.
        let (actor, session_id) = actor_with_session();

        // When handling a Created event.
        actor.handle_created(&DiscordThreadCreated {
            session_id: session_id.clone(),
            title: "My Cool Session".to_owned(),
        });

        // Then the session's last history entry is a System entry with the title.
        let guard = actor.state.read();
        let last = guard.session(&session_id).history().last().expect("entry");
        assert!(matches!(last.kind, ChatEntryKind::System(_)));
        assert!(last.text().contains("My Cool Session"));
    }

    /// A `Failed(AlreadyBound)` event pushes an error entry.
    #[test]
    fn failed_already_bound_pushes_error_entry() {
        // Given an actor with one session.
        let (actor, session_id) = actor_with_session();

        // When handling a Failed(AlreadyBound) event.
        actor.handle_failed(&DiscordThreadCreateFailed {
            session_id: session_id.clone(),
            reason: CreateThreadReason::AlreadyBound,
        });

        // Then the session's last history entry is an Error entry.
        let guard = actor.state.read();
        let last = guard.session(&session_id).history().last().expect("entry");
        assert!(matches!(last.kind, ChatEntryKind::Error(_)));
    }

    /// A result for a session that doesn't exist is dropped, not panicked.
    #[test]
    fn result_for_missing_session_is_dropped() {
        // Given a state with no sessions.
        let state = State::new(AppState::default());
        let session_id = SessionId::new();

        // When pushing an entry for a session that doesn't exist.
        push_entry(&state, &session_id, ChatEntry::system("nope"));

        // Then no panic occurred (reaching here is the assertion).
    }
}
