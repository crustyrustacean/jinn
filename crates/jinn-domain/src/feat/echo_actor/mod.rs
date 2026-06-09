//! Echo actor — reference actor for jinn.
//!
//! Echoes user messages as ALL CAPS actor entries after a 1-second delay.
//! Receives [`ChatEntrySubmitted`] events from the message bus and publishes
//! [`PushChatEntry`] commands back to the bus.

use std::time::Duration;

use kameo::prelude::{Actor, ActorRef, Context, Message};
use kameo_actors::message_bus::{Publish, Register};

use crate::common::services::bus_service::BusService;
use crate::feat::chat_input::protocol::command::PushChatEntry;
use crate::feat::chat_input::protocol::event::ChatEntrySubmitted;
use crate::protocol::{ChatEntry, ChatEntryKind};

/// Dependencies for spawning an [`EchoActor`].
pub struct EchoActorDeps {
    /// Reference to the message bus for publishing commands.
    pub bus: BusService,
}

/// Reference echo actor that echoes user messages back as actor entries.
pub struct EchoActor {
    bus: BusService,
}

impl Actor for EchoActor {
    type Args = EchoActorDeps;
    type Error = kameo::error::Infallible;

    async fn on_start(
        deps: Self::Args,
        actor_ref: ActorRef<Self>,
    ) -> Result<Self, Self::Error> {
        // Register to receive ChatEntrySubmitted events from the bus.
        let recipient = actor_ref.recipient::<ChatEntrySubmitted>();
        let _ = deps
            .bus
            .actor_ref()
            .tell(Register(recipient))
            .await
            .inspect_err(|e| {
                tracing::error!(err = ?e, "echo actor failed to register on bus");
            });

        Ok(Self { bus: deps.bus })
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
        let _ = self
            .bus
            .actor_ref()
            .tell(Publish(push))
            .await
            .inspect_err(|e| {
                tracing::error!(err = ?e, "echo actor failed to publish PushChatEntry");
            });
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

    use kameo::actor::Spawn;
    use super::*;
    use crate::protocol::SessionId;

    /// Query message to retrieve collected messages from a Recorder.
    pub struct GetRecorded;

    /// A simple recorder actor that collects messages of type M.
    pub struct Recorder<M> {
        messages: Vec<M>,
    }

    impl<M: Send + 'static> Actor for Recorder<M> {
        type Args = ();
        type Error = kameo::error::Infallible;

        async fn on_start(
            _args: Self::Args,
            _actor_ref: ActorRef<Self>,
        ) -> Result<Self, Self::Error> {
            Ok(Self {
                messages: Vec::new(),
            })
        }
    }

    impl<M: Clone + Send + 'static> Message<M> for Recorder<M> {
        type Reply = ();

        async fn handle(
            &mut self,
            msg: M,
            _ctx: &mut Context<Self, Self::Reply>,
        ) -> Self::Reply {
            self.messages.push(msg);
        }
    }

    impl<M: Clone + Send + 'static> Message<GetRecorded> for Recorder<M> {
        type Reply = Vec<M>;

        async fn handle(
            &mut self,
            _msg: GetRecorded,
            _ctx: &mut Context<Self, Self::Reply>,
        ) -> Self::Reply {
            self.messages.clone()
        }
    }

    #[tokio::test]
    async fn echo_actor_publishes_uppercase_push_chat_entry() {
        // Given an echo actor and a recorder actor, both registered on the bus.
        let bus = kameo_actors::message_bus::MessageBus::new(kameo_actors::DeliveryStrategy::BestEffort);
        let bus_ref = kameo::prelude::Spawn::spawn(bus);
        let bus_service = BusService::new(bus_ref.clone());

        let _echo = EchoActor::spawn(EchoActorDeps {
            bus: bus_service.clone(),
        });

        // Give the bus time to process the echo actor's registration.
        tokio::time::sleep(Duration::from_millis(50)).await;

        let recorder = Recorder::<PushChatEntry>::spawn(());
        bus_ref
            .tell(Register(recorder.clone().recipient::<PushChatEntry>()))
            .await
            .expect("register recorder");
        // Give the bus time to process the recorder's registration.
        tokio::time::sleep(Duration::from_millis(50)).await;

        // When publishing a ChatEntrySubmitted with a user message.
        let session_id = SessionId::new();
        bus_ref
            .tell(Publish(ChatEntrySubmitted {
                session_id: session_id.clone(),
                entry: ChatEntry::user("hello world"),
            }))
            .await
            .expect("publish ChatEntrySubmitted");

        // Then the recorder received a PushChatEntry with "HELLO WORLD".
        // Wait for the 1-second delay.
        tokio::time::sleep(Duration::from_secs(2)).await;
        let recorded: Vec<PushChatEntry> = recorder.ask(GetRecorded).await.expect("get recorded messages");

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
        // Given an echo actor and a recorder actor, both registered on the bus.
        let bus = kameo_actors::message_bus::MessageBus::new(kameo_actors::DeliveryStrategy::BestEffort);
        let bus_ref = kameo::prelude::Spawn::spawn(bus);
        let bus_service = BusService::new(bus_ref.clone());

        let _echo = EchoActor::spawn(EchoActorDeps {
            bus: bus_service.clone(),
        });

        let recorder = Recorder::<PushChatEntry>::spawn(());
        bus_ref
            .tell(Register(recorder.clone().recipient::<PushChatEntry>()))
            .await
            .expect("register recorder");

        // When publishing a ChatEntrySubmitted with a system (non-user) entry.
        bus_ref
            .tell(Publish(ChatEntrySubmitted {
                session_id: SessionId::new(),
                entry: ChatEntry::system("system message"),
            }))
            .await
            .expect("publish ChatEntrySubmitted");

        // Then no PushChatEntry was published (echo ignores non-user entries).
        tokio::time::sleep(Duration::from_millis(500)).await;
        let recorded: Vec<PushChatEntry> = recorder.ask(GetRecorded).await.expect("get recorded messages");

        assert!(recorded.is_empty(), "expected no PushChatEntry for system entry");
    }
}
