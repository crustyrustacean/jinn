//! Echo actor — reference actor for nullslop.
//!
//! Implements [`Actor`] for in-memory hosting. Echoes user messages as ALL CAPS
//! actor entries after a 1-second delay. Lifecycle announcements
//! (`ActorStarted`, `ActorShutdownCompleted`) are sent via the `ActorContext`
//! helpers, which are automatically triggered by host-broadcast lifecycle events.

use std::time::Duration;

use crate::common::actor::{Actor, ActorContext, ActorEnvelope, NoDirectMsg};

use crate::feat::chat_input::protocol::command::PushChatEntry;
use crate::feat::chat_input::protocol::event::ChatEntrySubmitted;
use crate::protocol::{ChatEntry, ChatEntryKind, Command, Event};

/// Reference echo actor that echoes user messages back as actor entries.
pub struct EchoActor;

impl Actor for EchoActor {
    type Message = NoDirectMsg;

    fn activate(ctx: &mut ActorContext) -> Self {
        ctx.subscribe_event::<ChatEntrySubmitted>();

        Self
    }

    async fn handle(&mut self, msg: ActorEnvelope<NoDirectMsg>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Event(event) => Self::process_event(&event, ctx).await,
            ActorEnvelope::Command(_) | ActorEnvelope::System(_) => {}
        }
    }
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
