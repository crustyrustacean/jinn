//! Projector actor — empty shell pending deletion in Phase 4.

use nullslop_actor::{Actor, ActorContext, ActorEnvelope, SystemMessage};

/// Direct message type (unused).
pub enum ProjectorDirectMsg {}

/// Pure event→state projector.
///
/// Empty shell — all handlers have been migrated to domain-specific actors.
pub struct ProjectorActor;

impl Actor for ProjectorActor {
    type Message = ProjectorDirectMsg;

    fn activate(ctx: &mut ActorContext) -> Self {
        ctx.set_description("Empty shell — migrating to domain actors");
        Self
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::System(SystemMessage::ApplicationReady) => {
                ctx.announce_started();
            }
            ActorEnvelope::System(SystemMessage::ApplicationShuttingDown) => {
                ctx.announce_shutdown_completed();
            }
            ActorEnvelope::Command(_)
            | ActorEnvelope::Event(_)
            | ActorEnvelope::Direct(_)
            | ActorEnvelope::Shutdown => {}
        }
    }

    async fn shutdown(self) {}
}
