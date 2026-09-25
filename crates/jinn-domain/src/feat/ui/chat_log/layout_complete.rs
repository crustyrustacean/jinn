//! [`LayoutCompletionActor`] — applies measured chat log line counts and ends the load.
//!
//! The session load guard is the chat log's "still loading" indication, and it
//! used to be cleared the moment a session was read from disk. That made the
//! very next frame run the whole layout pass, which is what froze the UI on a
//! large session. Now the guard spans both phases: the load leaves it standing,
//! and this actor clears it once the counts have been measured and stored.
//!
//! A result for a session that is no longer on screen is discarded — those
//! counts would land in a cache nobody reads. A result for the session that *is*
//! on screen but was measured at a stale width is also discarded, because the
//! counts would be wrong at the width actually in use. Either way the guard is
//! cleared: a discarded result degrades to a slow frame, and leaving the
//! indication up would strand the user on a spinner forever.

use error_stack::Report;
use jinn_chat_log_view::chat_log::{ContentIdentity, MeasuredLineCount};
use jinn_chat_log_view_msg::ChatLogLayoutComputed;
use trouper::actor::{ActorPath, MsgHandler, ServiceActor};
use trouper::context::MsgCtx;
use trouper::registry::RegistryError;

use crate::common::state::State;

/// Static path the layout completion actor spawns at (one per process).
pub const LAYOUT_COMPLETION_PATH: &str = "jinn.chat_log.layout.completion";

/// Dependencies for [`LayoutCompletionActor`].
#[derive(Clone)]
pub struct LayoutCompletionActorDeps {
    /// Shared application state, holding the line cache and the load guard.
    pub state: State,
}

/// What the completion actor did with a measured result.
///
/// Exposed so a discarded result is observable rather than silent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutApplied {
    /// The counts were stored and the load guard was cleared.
    Applied,
    /// The counts were for a session that is no longer active, so they were
    /// dropped. The load guard was left alone — the active session's own
    /// measurement still owns it.
    DiscardedInactive,
    /// The counts were measured at a width the chat log is no longer using.
    /// The load guard was cleared, so the next frame measures inline.
    DiscardedStaleWidth,
}

/// Stores measured line counts and clears the session load guard.
pub struct LayoutCompletionActor {
    /// Shared application state.
    state: State,
}

impl ServiceActor for LayoutCompletionActor {
    #[expect(
        clippy::unused_async_trait_impl,
        reason = "ServiceActor::start is async by trait contract"
    )]
    async fn start(_args: &trouper::json::Json) -> Result<Self, Report<RegistryError>> {
        // Never called: spawned via `spawn`'s start_with (typed deps can't
        // ride the JSON args).
        Err(Report::new(RegistryError::InvalidSpec)
            .attach("LayoutCompletionActor spawns via start_with"))
    }
}

impl LayoutCompletionActor {
    /// Spawns the completion actor.
    ///
    /// # Panics
    ///
    /// Panics if the actor's path is already taken — a wiring bug.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "port convention: spawn takes owned deps and clones into start_with"
    )]
    pub fn spawn(
        system: &trouper::system::ActorSystem,
        deps: LayoutCompletionActorDeps,
    ) -> ActorPath {
        let path = ActorPath::new(LAYOUT_COMPLETION_PATH);
        trouper::builder::spawn_service_builder::<Self>(system)
            .at(path.clone())
            .start_with({
                let deps = deps.clone();
                move || {
                    let deps = deps.clone();
                    Box::pin(async move { Ok(Self { state: deps.state }) })
                }
            })
            .handles::<ChatLogLayoutComputed>()
            .mailbox(64, trouper::inbox::OverloadPolicy::Block)
            .start();
        path
    }

    /// Stores a result and clears the guard if it belongs to the active session.
    pub(crate) fn apply(&self, computed: &ChatLogLayoutComputed) -> LayoutApplied {
        let outcome = {
            let state = self.state.read();
            if state.session.active_session_id() != &computed.session_id {
                // Someone else's measurement. Their load guard is not ours to
                // clear, and the counts would land in a cache nobody reads.
                return LayoutApplied::DiscardedInactive;
            }
            let session = state.session.get(&computed.session_id);
            if session.is_some_and(|s| s.content_width() != computed.content_width) {
                // The chat log was resized while the measurement ran. These
                // counts describe a width nothing is rendering at, so the
                // next frame must measure again.
                Outcome::Stale
            } else {
                self.store_counts(&state, computed);
                Outcome::Current
            }
        };

        // Clearing the guard is a write, so it happens through the session
        // projection rather than on the read guard. It is done for every
        // outcome that belongs to the active session: the loading
        // indication must not outlive a measurement, even a discarded one,
        // or the user would sit in front of a spinner forever.
        if matches!(outcome, Outcome::Current | Outcome::Stale) {
            self.state
                .with_session(|view| view.session.map().clear_load());
        }
        match outcome {
            Outcome::Current => LayoutApplied::Applied,
            Outcome::Stale => LayoutApplied::DiscardedStaleWidth,
        }
    }

    /// Writes the measured counts into the shared line cache.
    ///
    /// The counts arrive with the identity the worker measured against, so
    /// they are stored verbatim: re-deriving the hashes here would walk
    /// every entry's content a second time and cost as much as the
    /// measurement itself.
    fn store_counts(
        &self,
        state: &crate::common::app_state::AppState,
        computed: &ChatLogLayoutComputed,
    ) {
        let measured: Vec<MeasuredLineCount> = computed
            .counts
            .iter()
            .map(|count| MeasuredLineCount {
                id: count.entry_id.clone(),
                content: ContentIdentity {
                    signature: count.signature,
                    fingerprint: count.fingerprint,
                },
                is_expanded: count.is_expanded,
                variant: count.variant,
                wrapped_count: count.wrapped_count,
            })
            .collect();
        state
            .frontend
            .caches
            .entry_line_cache
            .write()
            .insert_counts(&measured, computed.content_width);
    }
}

/// Whether the active session's own measurement produced usable counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// Counts describe the live chat log and were stored.
    Current,
    /// Counts were measured at a width the chat log is no longer using.
    Stale,
}

impl MsgHandler<ChatLogLayoutComputed> for LayoutCompletionActor {
    async fn handle(&mut self, msg: &ChatLogLayoutComputed, _ctx: &mut MsgCtx<'_>) {
        let applied = self.apply(msg);
        if applied != LayoutApplied::Applied {
            tracing::debug!(
                session_id = %msg.session_id,
                content_width = msg.content_width,
                ?applied,
                "discarded chat log layout counts"
            );
        }
    }
}
