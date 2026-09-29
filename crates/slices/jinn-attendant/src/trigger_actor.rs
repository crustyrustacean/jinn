//! The trigger actor — fires attendants when a parent's turn completes.

use jinn_attendant_msg::AttendantTrigger;
use jinn_chat_input_msg::EnqueueUserMessage;
use jinn_inference_msg::CancelStream;
use jinn_kernel::Services;
use jinn_kernel::common::state::State;
use jinn_session_msg::{PhaseKind, TurnCompleted, TurnOutcome};
use trouper::actor::{ActorPath, MsgHandler, ServiceActor};
use trouper::context::MsgCtx;
use trouper::registry::RegistryError;
use trouper::system::ActorSystem;

use crate::activation;

/// Where the trigger actor lives on the trouper system.
const ATTENDANT_TRIGGER_PATH: &str = "jinn.domain/attendant-trigger";

/// Dependencies for [`AttendantTriggerActor`].
#[derive(Clone)]
pub struct AttendantTriggerActorDeps {
    /// Application-wide runtime services (bus publish).
    pub services: Services,
    /// Shared application state — the live session map lives here.
    pub state: State,
}

/// Fires attendants when their parent's turn completes successfully.
pub struct AttendantTriggerActor {
    services: Services,
    state: State,
}

impl ServiceActor for AttendantTriggerActor {
    #[expect(
        clippy::unused_async_trait_impl,
        reason = "ServiceActor::start is async by trait contract"
    )]
    async fn start(
        _args: &trouper::json::Json,
    ) -> Result<Self, error_stack::Report<RegistryError>> {
        // Never called: the spawn helper injects the deps via `start_with`
        // (Services carries typed handles that cannot ride JSON args).
        Err(error_stack::Report::new(RegistryError::InvalidSpec)
            .attach("AttendantTriggerActor is spawned via start_with"))
    }
}

/// The messages firing one attendant produces.
struct Fired {
    /// Cancel the attendant's in-flight turn, if it was busy.
    cancel: Option<CancelStream>,
    /// Dispatch the attendant's run.
    dispatch: Option<EnqueueUserMessage>,
    /// Write the session back if a `Reset` excluded something, so the
    /// exclusions outlive a restart instead of silently reverting.
    persist: bool,
}

impl AttendantTriggerActor {
    /// Spawns the actor and subscribes it to `TurnCompleted`.
    ///
    /// The subscription is the readiness point: publishes after this call
    /// resolves cannot be missed.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "port convention: spawn takes owned deps and clones into start_with"
    )]
    pub fn spawn(system: &ActorSystem, deps: AttendantTriggerActorDeps) -> ActorPath {
        let path = ActorPath::new(ATTENDANT_TRIGGER_PATH);
        trouper::builder::spawn_service_builder::<Self>(system)
            .at(path.clone())
            .start_with({
                let deps = deps.clone();
                move || {
                    let deps = deps.clone();
                    Box::pin(async move {
                        Ok(Self {
                            services: deps.services,
                            state: deps.state,
                        })
                    })
                }
            })
            .handles::<TurnCompleted>()
            .start();
        path
    }

    /// Runs the trigger for one completed turn.
    ///
    /// Only a `Succeeded` outcome fires attendants — an errored or cancelled
    /// turn is not something to verify against, and silently not firing is
    /// the visible-conservatism the design leans on. The query runs only on
    /// a live event, so an attendant created after its parent completed
    /// never fires retroactively.
    ///
    /// A turn that was itself started by automation does **not** fire the
    /// completing session's own attendants. That suppression is what stops
    /// an attendant that notified its parent from bouncing a mutually
    /// triggering exchange back and forth unattended: the child's dispatch
    /// marks the child automated, and when the child's turn finishes the
    /// child must not wake anything of its own. The marker is set at
    /// dispatch and stays set until the *next* turn begins, so it is
    /// observable at the moment the outcome is decided.
    fn on_turn_completed(&self, event: &TurnCompleted) {
        if event.outcome != TurnOutcome::Succeeded {
            return;
        }
        if self.was_automated(&event.session_id) {
            return;
        }
        let attendants = self.attendants_of(&event.session_id);
        for attendant_id in attendants {
            if let Some(fired) = self.fire(&attendant_id) {
                if let Some(cancel) = fired.cancel {
                    self.publish(cancel);
                }
                if let Some(dispatch) = fired.dispatch {
                    self.publish(dispatch);
                }
                if fired.persist {
                    self.publish(jinn_session_store_msg::PersistSession {
                        session_id: attendant_id,
                    });
                }
            }
        }
    }

    /// Whether the session's just-finished turn was started by automation.
    fn was_automated(&self, session_id: &jinn_core_types::SessionId) -> bool {
        {
            let state = self.state.read();
            state
                .session
                .get(session_id)
                .is_some_and(jinn_session_state::ChatSessionState::is_turn_automated)
        }
    }

    /// Every loaded attendant watching `parent`, by live query.
    ///
    /// A child references its parent; the parent holds no list. Creation
    /// order is irrelevant — this is why an attendant created after its
    /// parent finished still shows up the *next* time the parent completes,
    /// and why nothing fires for it in between.
    fn attendants_of(
        &self,
        parent: &jinn_core_types::SessionId,
    ) -> Vec<jinn_core_types::SessionId> {
        {
            let state = self.state.read();
            state
                .session
                .iter()
                .filter(|(_, session)| {
                    session.is_attendant()
                        && session.attendant_trigger() == AttendantTrigger::ParentCompleted
                        && session.parent_session().as_ref() == Some(parent)
                })
                .map(|(id, _)| id.clone())
                .collect()
        }
    }

    /// Prepares one attendant for a run.
    ///
    /// The sequence, in order:
    ///
    /// 1. `Seed` activation is inert — the user is still composing its
    ///    instructions, and firing against half-written pins is the exact
    ///    failure the mode exists to prevent.
    /// 2. A busy attendant's own turn is cancelled. That is a single-session
    ///    cancel: a re-trigger supersedes *this* attendant's work, and its
    ///    descendants still answer a question this attendant exists to read.
    ///    It is not the confirmed-cancel cascade — a trigger does not know
    ///    which descendant the user would want cancelled, so that stays a
    ///    manual `R`.
    /// 3. `Reset` activation force-excludes every non-pinned entry, so the
    ///    model sees the pins alone. The changed session is persisted.
    /// 4. A prior report is injected through the seed template, and the
    ///    resulting entry is dispatched as a fresh user turn.
    fn fire(&self, attendant_id: &jinn_core_types::SessionId) -> Option<Fired> {
        {
            let mut state = self.state.write();
            let session = state.session.get_mut(attendant_id)?;
            if !session.attendant_activation().is_dispatchable() {
                return None;
            }

            let cancel = (session.phase() != PhaseKind::Idle).then(|| CancelStream {
                session_id: attendant_id.clone(),
            });

            // A trigger respects the mode: it seeds for `Seed`/`Reset` and
            // carries the existing context for `Preserve`, so an unattended
            // fire never injects a message the user did not ask for.
            let (entry, reset) = activation::prepare_trigger_run(session);
            let dispatch = entry.map(|entry| {
                session.mark_turn_automated();
                EnqueueUserMessage {
                    session_id: attendant_id.clone(),
                    entry,
                }
            });

            Some(Fired {
                cancel,
                dispatch,
                persist: !reset.is_empty(),
            })
        }
    }

    /// Publishes one message onto the bus.
    fn publish<M>(&self, message: M)
    where
        M: jinn_slices::BusMessage
            + trouper::schema::Schema
            + serde::Serialize
            + Clone
            + Send
            + Sync
            + trouper::envelope::PayloadValue,
    {
        let bus = self.services.bus.clone();
        tokio::spawn(async move {
            bus.publish(message).await;
        });
    }
}

impl MsgHandler<TurnCompleted> for AttendantTriggerActor {
    async fn handle(&mut self, msg: &TurnCompleted, _ctx: &mut MsgCtx<'_>) {
        self.on_turn_completed(msg);
    }
}
