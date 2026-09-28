//! The trigger actor — fires attendants when a parent's turn completes.

use jinn_kernel::Services;
use jinn_kernel::common::state::State;
use jinn_session_msg::TurnCompleted;
use jinn_session_msg::TurnOutcome;
use trouper::actor::{ActorPath, MsgHandler, ServiceActor};
use trouper::context::MsgCtx;
use trouper::registry::RegistryError;
use trouper::system::ActorSystem;

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
    #[allow(dead_code, reason = "Phase 4 consumes both; the skeleton subscribes only")]
    services: Services,
    #[allow(dead_code, reason = "Phase 4 consumes both; the skeleton subscribes only")]
    state: State,
}

impl ServiceActor for AttendantTriggerActor {
    async fn start(_args: &trouper::json::Json) -> Result<Self, error_stack::Report<RegistryError>> {
        // Never called: the spawn helper injects the deps via `start_with`
        // (Services carries typed handles that cannot ride JSON args).
        Err(error_stack::Report::new(RegistryError::InvalidSpec)
            .attach("AttendantTriggerActor is spawned via start_with"))
    }
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
    /// turn is not something to verify against. See Phase 4 for the full
    /// fire sequence.
    fn on_turn_completed(&self, event: &TurnCompleted) {
        if event.outcome != TurnOutcome::Succeeded {
            return;
        }
        tracing::debug!(
            session_id = %event.session_id,
            "attendant trigger observed a succeeded turn"
        );
    }
}

impl MsgHandler<TurnCompleted> for AttendantTriggerActor {
    async fn handle(&mut self, msg: &TurnCompleted, _ctx: &mut MsgCtx<'_>) {
        self.on_turn_completed(msg);
    }
}
