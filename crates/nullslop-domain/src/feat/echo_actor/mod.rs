//! Echo actor — reference actor for nullslop.
//!
//! Implements [`Actor`] for in-memory hosting. Echoes user messages as ALL CAPS
//! actor entries after a 1-second delay. Lifecycle announcements
//! (`ActorStarted`, `ActorShutdownCompleted`) are sent via the `ActorContext`
//! helpers, which are automatically triggered by host-broadcast lifecycle events.

use std::sync::Arc;
use std::time::Duration;

use crate::common::actor::{
    Actor, ActorContext, ActorEnvelope, ActorRef, MessageSink, SystemMessage,
};
use crate::common::actor_host::{ActorSpawnResult, spawn_actor_impl};

use crate::feat::chat_input::protocol::command::PushChatEntry;
use crate::feat::chat_input::protocol::event::ChatEntrySubmitted;
use crate::protocol::{ChatEntry, ChatEntryKind, Command, Event};

/// Direct message type for the echo actor.
/// Currently unused — the echo actor only responds to bus events.
pub enum EchoDirectMsg {}

/// Reference echo actor that echoes user messages back as actor entries.
pub struct EchoActor;

impl Actor for EchoActor {
    type Message = EchoDirectMsg;

    fn activate(ctx: &mut ActorContext) -> Self {
        ctx.subscribe_event::<ChatEntrySubmitted>();

        Self
    }

    async fn handle(&mut self, msg: ActorEnvelope<EchoDirectMsg>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::System(SystemMessage::ApplicationShuttingDown) => {
                ctx.announce_shutdown_completed();
            }
            ActorEnvelope::Event(event) => Self::process_event(&event, ctx).await,
            ActorEnvelope::Command(_) | ActorEnvelope::Direct(_) | ActorEnvelope::Shutdown => {}
        }
    }

    async fn shutdown(self) {}
}

/// Spawns the echo actor on the given tokio runtime.
///
/// Returns the actor reference and spawn result for routing registration.
pub fn spawn(
    sink: Arc<dyn MessageSink>,
    handle: &tokio::runtime::Handle,
) -> (ActorRef<EchoDirectMsg>, ActorSpawnResult) {
    let (tx, rx) = kanal::unbounded::<ActorEnvelope<EchoDirectMsg>>();
    let actor_ref = ActorRef::new(tx);
    let mut ctx = ActorContext::new("echo", sink);
    let actor = EchoActor::activate(&mut ctx);
    let result = spawn_actor_impl("echo", actor, &actor_ref, rx, ctx, handle);
    (actor_ref, result)
}

impl EchoActor {
    /// Processes an incoming event, echoing user messages as ALL CAPS actor entries.
    async fn process_event(event: &Event, ctx: &ActorContext) {
        match event {
            Event::ChatEntrySubmitted {
                payload:
                    ChatEntrySubmitted {
                        session_id,
                        entry:
                            ChatEntry {
                                kind: ChatEntryKind::User(text),
                                ..
                            },
                        ..
                    },
            } => {
                tokio::time::sleep(Duration::from_secs(1)).await;
                if let Err(e) = ctx.send_command(Command::PushChatEntry {
                    payload: PushChatEntry {
                        session_id: session_id.clone(),
                        entry: ChatEntry::actor("echo", text.to_uppercase()),
                    },
                }) {
                    tracing::error!(err = ?e, "echo actor failed to send command");
                }
            }
            _ => {}
        }
    }
}
