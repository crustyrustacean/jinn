//! The trigger actor — fires attendants when a parent's turn completes.

use jinn_attendant_msg::AttendantTrigger;
use jinn_chat_input_msg::EnqueueUserMessage;
use jinn_inference_msg::CancelStream;
use jinn_kernel::Services;
use jinn_kernel::common::state::State;
use jinn_session_msg::{TurnCompleted, TurnOutcome};
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

    /// Every loaded attendant in the subtree rooted at `parent`, by live query.
    ///
    /// A child references its parent; the parent holds no list. Creation
    /// order is irrelevant — this is why an attendant created after its
    /// parent finished still shows up the *next* time the parent completes,
    /// and why nothing fires for it in between.
    ///
    /// The walk is recursive, not one hop: an attendant created under another
    /// attendant is still under the parent it reports to, so the root's
    /// completion is what the user's question completed. Fork and user
    /// sessions are boundaries — they are not attendants, and nothing hangs
    /// off them by this rule.
    fn attendants_of(
        &self,
        parent: &jinn_core_types::SessionId,
    ) -> Vec<jinn_core_types::SessionId> {
        let mut found = Vec::new();
        // Seeded with the completed session: a child that links back up to
        // it must be recognised as already walked, or a cyclic
        // `parent_session` chain recurses forever.
        let mut visited = std::collections::HashSet::new();
        visited.insert(parent.clone());
        self.collect_attendants_under(parent, &mut visited, &mut found);
        found
    }

    /// Appends every attendant beneath `session`, depth first, skipping any
    /// session already walked.
    ///
    /// The `visited` set is seeded by the caller with the completed session,
    /// so a cycle in `parent_session` links terminates instead of recursing
    /// until the stack gives out.
    fn collect_attendants_under(
        &self,
        session: &jinn_core_types::SessionId,
        visited: &mut std::collections::HashSet<jinn_core_types::SessionId>,
        found: &mut Vec<jinn_core_types::SessionId>,
    ) {
        let children: Vec<jinn_core_types::SessionId> = {
            let state = self.state.read();
            state
                .session
                .iter()
                .filter(|(_, candidate)| {
                    candidate.is_attendant()
                        && candidate.attendant_trigger() == AttendantTrigger::ParentCompleted
                        && candidate.parent_session().as_ref() == Some(session)
                })
                .map(|(id, _)| id.clone())
                .collect()
        };
        for child in children {
            if !visited.insert(child.clone()) {
                continue;
            }
            found.push(child.clone());
            self.collect_attendants_under(&child, visited, found);
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
            // Whether a fire is configured to run at all. Seed is the
            // attendant being composed, so nothing dispatches for it yet.
            if !session
                .attendant_trigger()
                .is_enabled_for(session.attendant_activation())
            {
                return None;
            }

            // A busy attendant is NOT superseded by a trigger: its in-flight
            // turn is real work the user is waiting on, and nothing in a
            // parent-completed fire asks for it to be thrown away. The
            // enqueue handler queues anything arriving while a session is
            // Sending/Streaming, so the seeded entry below waits its turn and
            // runs when the current one finishes.
            //
            // Publishing `CancelStream` here instead produced two visible
            // faults: the running turn was aborted, leaving a "Cancelled"
            // entry in the attendant's history, and the seeded turn sat in the
            // queue where a later cancel drained it into the input box as a
            // templated draft the user never sent.
            //
            // `R` is the opposite case and does cancel: there the user asked
            // for this question to be asked again.
            let cancel: Option<CancelStream> = None;

            // The mode decides what the run sees, not whether it happens.
            // Every mode that got past the guard above sends the template.
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
