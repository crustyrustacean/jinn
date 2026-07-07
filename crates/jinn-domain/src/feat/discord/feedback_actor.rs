//! Discord to-thread feedback actor.
//!
//! Closes the loop on `gdc` ("to-thread"). When the gateway finishes (or fails)
//! creating a Discord forum thread for a session, it publishes
//! [`DiscordThreadCreated`] / [`DiscordThreadCreateFailed`] onto the bus. This
//! kameo actor subscribes to both and writes a [`ChatEntry`] into the targeted
//! session's history so the user sees the outcome inline where they invoked
//! `gdc`, not just in Discord.
//!
//! The actor owns a clone of [`State`] and is the sole writer of these
//! confirmation/error entries. It uses the fallible `try_session_mut` accessor:
//! a result event arriving for a session that was closed/archived in the
//! meantime is dropped with a debug log rather than panicking.
//!
//! Spawned only when `[discord] enabled = true`, with `RestartPolicy::Never`
//! (it's a stateless forwarder; if it dies, the user just won't see the inline
//! confirmation — Discord itself still works).

use kameo::prelude::{Actor, ActorRef, Context, Message};

use crate::SessionId;
use crate::common::actor_deps::ActorDeps;
use crate::common::state::State;
use crate::feat::discord::protocol::{
    CreateThreadReason, DiscordThreadCreateFailed, DiscordThreadCreated, ForumChannelError,
};
use crate::feat::session::chat_entry::ChatEntry;

/// The feedback actor.
pub struct DiscordFeedbackActor {
    /// Shared application state — writes the confirmation/error `ChatEntry`.
    state: State,
}

/// Dependencies for [`DiscordFeedbackActor`].
#[derive(Clone)]
pub struct DiscordFeedbackActorDeps {
    /// Universal actor dependencies (bus subscription handle).
    pub deps: ActorDeps,
    /// Shared application state.
    pub state: State,
}

impl Actor for DiscordFeedbackActor {
    type Args = DiscordFeedbackActorDeps;
    type Error = std::convert::Infallible;

    async fn on_start(args: Self::Args, actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        args.deps
            .subscribe(actor_ref.clone().recipient::<DiscordThreadCreated>())
            .await;
        args.deps
            .subscribe(actor_ref.recipient::<DiscordThreadCreateFailed>())
            .await;
        Ok(Self { state: args.state })
    }
}

impl Message<DiscordThreadCreated> for DiscordFeedbackActor {
    type Reply = ();

    async fn handle(&mut self, msg: DiscordThreadCreated, _ctx: &mut Context<Self, Self::Reply>) {
        self.handle_created(&msg);
    }
}

impl Message<DiscordThreadCreateFailed> for DiscordFeedbackActor {
    type Reply = ();

    async fn handle(
        &mut self,
        msg: DiscordThreadCreateFailed,
        _ctx: &mut Context<Self, Self::Reply>,
    ) {
        self.handle_failed(&msg);
    }
}

impl DiscordFeedbackActor {
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
        CreateThreadReason::AlreadyBound => {
            "Can't continue in Discord: this session is already in a Discord \
             thread — continue there."
                .to_owned()
        }
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
        CreateThreadReason::MappingWriteFailed => {
            "Discord thread was created, but jinn couldn't record the binding — \
             the thread exists but won't receive replies. See the logs."
                .to_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, reason = "test code")]
    use super::*;
    use crate::common::app_state::AppState;
    use crate::feat::discord::protocol::CreateThreadReason;
    use crate::feat::session::chat_entry::ChatEntryKind;
    use crate::protocol::SessionId;

    fn actor_with_session() -> (DiscordFeedbackActor, SessionId) {
        let state = State::new(AppState::default());
        let session_id = SessionId::new();
        // Seed the session so `try_session_mut` finds it.
        state.write().session_mut_or_create(&session_id);
        let actor = DiscordFeedbackActor { state };
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
