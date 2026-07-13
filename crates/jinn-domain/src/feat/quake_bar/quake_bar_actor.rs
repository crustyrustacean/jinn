//! Quake bar actor — the sole owner/writer of the command log.
//!
//! Subscribes to [`SubmitQuakeBarCommand`] and appends the submitted line to
//! [`QuakeBarState::log`](super::state::QuakeBarState::log). Keeping this actor
//! as the only writer of the log lets future quake-specific debug commands and
//! event subscriptions funnel through one mutator.

use kameo::prelude::{Actor, ActorRef, Context, Message};

use crate::common::actor_deps::ActorDeps;
use crate::common::state::State;
use crate::feat::quake_bar::command::SubmitQuakeBarCommand;

/// Owns the quake bar command log.
///
/// The single subscriber to [`SubmitQuakeBarCommand`]; the only writer of
/// [`QuakeBarState::log`](super::state::QuakeBarState::log).
pub struct QuakeBarActor {
    /// Shared application state.
    state: State,
    /// Capability to write `frontend.quake_bar`.
    cap: crate::common::tcaps::frontend::FrontendCap,
}

/// Dependencies for spawning a [`QuakeBarActor`].
#[derive(Clone)]
pub struct QuakeBarActorDeps {
    /// Universal actor dependencies (bus, services, etc.).
    pub deps: ActorDeps,
    /// Shared application state.
    pub state: State,
    /// Capability to write `frontend.quake_bar`.
    pub cap: crate::common::tcaps::frontend::FrontendCap,
}

impl Actor for QuakeBarActor {
    type Args = QuakeBarActorDeps;
    type Error = kameo::error::Infallible;

    async fn on_start(args: Self::Args, actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        args.deps
            .subscribe(actor_ref.recipient::<SubmitQuakeBarCommand>())
            .await;
        Ok(Self { state: args.state, cap: args.cap })
    }
}

impl Message<SubmitQuakeBarCommand> for QuakeBarActor {
    type Reply = ();

    async fn handle(
        &mut self,
        msg: SubmitQuakeBarCommand,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        self.apply_submit(msg);
    }
}

impl QuakeBarActor {
    /// Appends the submitted text to the command log.
    fn apply_submit(&self, msg: SubmitQuakeBarCommand) {
        use crate::common::tcaps::frontend::QuakeBarLogWrite;
        self.state
            .with_quake_bar(&self.cap, |ops| ops.push_log(msg.text));
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]

    use crate::common::app_state::AppState;
    use crate::common::state::State;
    use crate::feat::quake_bar::command::SubmitQuakeBarCommand;

    use super::QuakeBarActor;

    fn create_actor() -> (QuakeBarActor, State) {
        let state = State::new(AppState::default());
        let actor = QuakeBarActor {
            state: state.clone(),
            cap: crate::common::tcaps::mint::mint_frontend_cap(),
        };
        (actor, state)
    }

    #[test]
    fn submit_command_appends_text_to_log() {
        // Given a quake bar actor.
        let (actor, state) = create_actor();

        // When applying a SubmitQuakeBarCommand.
        actor.apply_submit(SubmitQuakeBarCommand {
            text: "hello".to_owned(),
        });

        // Then the text appears in the command log.
        let guard = state.read();
        assert_eq!(guard.frontend.quake_bar.log.len(), 1);
        assert_eq!(
            guard.frontend.quake_bar.log.visible_lines(5),
            &["hello".to_owned()]
        );
    }
}
