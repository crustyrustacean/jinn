//! Echo actor — reference actor for jinn.
//!
//! Echoes user messages as ALL CAPS actor entries after a 1-second delay.
//! Receives [`ChatEntrySubmitted`] events from the message bus and publishes
//! [`PushChatEntry`] commands back to the bus.

use std::time::Duration;

use kameo::prelude::{Actor, ActorRef, Context, Message};

use crate::common::actor_deps::{ActorDeps, BusPublish};
use crate::feat::chat_input::protocol::command::PushChatEntry;
use crate::feat::chat_input::protocol::event::ChatEntrySubmitted;
use crate::protocol::{ChatEntry, ChatEntryKind};

/// Dependencies for spawning an [`EchoActor`].
pub struct EchoActorDeps {
    /// Universal actor dependencies (bus, services, etc.).
    pub deps: ActorDeps,
}

/// Reference echo actor that echoes user messages back as actor entries.
pub struct EchoActor {
    deps: ActorDeps,
}

impl Actor for EchoActor {
    type Args = EchoActorDeps;
    type Error = kameo::error::Infallible;

    async fn on_start(
        args: Self::Args,
        actor_ref: ActorRef<Self>,
    ) -> Result<Self, Self::Error> {
        // Register to receive ChatEntrySubmitted events from the bus.
        args.deps.register(actor_ref.recipient::<ChatEntrySubmitted>()).await;

        Ok(Self { deps: args.deps })
    }
}

impl Message<ChatEntrySubmitted> for EchoActor {
    type Reply = ();

    async fn handle(
        &mut self,
        msg: ChatEntrySubmitted,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        let ChatEntrySubmitted {
            session_id,
            entry:
                ChatEntry {
                    kind: ChatEntryKind::User { display, .. },
                    ..
                },
            ..
        } = msg
        else {
            return;
        };

        tokio::time::sleep(Duration::from_secs(1)).await;

        let push = PushChatEntry {
            session_id,
            entry: ChatEntry::actor("echo", display.to_uppercase()),
        };
        self.publish(push).await;
    }
}

impl BusPublish for EchoActor {
    fn bus(&self) -> &crate::common::services::bus_service::BusService {
        self.deps.bus()
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

    use std::time::Duration;

    use super::*;
    use crate::common::bus::test_harness::{await_recorded, TestHarness};
    use crate::protocol::SessionId;

    #[tokio::test]
    async fn echo_actor_publishes_uppercase_push_chat_entry() {
        // Given an echo actor and a recorder, both registered on the bus.
        let harness = TestHarness::new().await;
        let _echo = harness.spawn_actor::<EchoActor>(EchoActorDeps { deps: harness.actor_deps() }).await;
        let recorder = harness.spawn_recorder::<PushChatEntry>().await;

        // When publishing a ChatEntrySubmitted with a user message.
        let session_id = SessionId::new();
        harness.publish(ChatEntrySubmitted {
            session_id: session_id.clone(),
            entry: ChatEntry::user("hello world"),
        }).await;

        // Then the recorder received a PushChatEntry with 'HELLO WORLD'.
        let recorded = await_recorded(&recorder, 1, Duration::from_secs(2)).await;

        assert_eq!(recorded.len(), 1, "expected exactly one PushChatEntry");

        let entry = &recorded[0];
        assert_eq!(entry.session_id, session_id);
        let ChatEntryKind::Actor { source, text } = &entry.entry.kind else {
            panic!("expected Actor entry kind");
        };
        assert_eq!(source, "echo");
        assert_eq!(text, "HELLO WORLD");
    }

    #[tokio::test]
    async fn echo_actor_ignores_non_user_entries() {
        // Given an echo actor and a recorder, both registered on the bus.
        let harness = TestHarness::new().await;
        let _echo = harness.spawn_actor::<EchoActor>(EchoActorDeps { deps: harness.actor_deps() }).await;
        let recorder = harness.spawn_recorder::<PushChatEntry>().await;

        // When publishing a ChatEntrySubmitted with a system (non-user) entry.
        harness.publish(ChatEntrySubmitted {
            session_id: SessionId::new(),
            entry: ChatEntry::system("system message"),
        }).await;

        // Then no PushChatEntry was published (echo ignores non-user entries).
        let recorded = await_recorded(&recorder, 1, Duration::from_millis(500)).await;
        assert!(recorded.is_empty(), "expected no PushChatEntry for system entry");
    }
}
