//! Discovery notifier — turns the coalesced [`SessionDiscoverySettled`] event
//! into a visible transient chat-history entry for that session.
//!
//! The notifier subscribes to [`SessionDiscoverySettled`] (emitted by the
//! [`DiscoveryCoordinatorActor`](crate::feat::discovery_coordinator::coordinator_actor::DiscoveryCoordinatorActor))
//! and posts a markdown summary as a `Transient` chat entry via `PushChatEntry`.
//! One entry per settled event — it fires only on the coalesced signal, never
//! on every scan or render tick.

use std::convert::Infallible;

use kameo::prelude::{Actor, ActorRef, Context, Message};
use kameo_actors::message_bus::{Publish, Register};

use crate::common::services::bus_service::BusService;
use crate::feat::chat_input::protocol::command::PushChatEntry;
use crate::feat::discovery_coordinator::SessionDiscoverySettled;
use crate::protocol::ChatEntry;

/// Dependencies injected at startup.
pub struct DiscoveryNotifierActorDeps {
    /// Bus for subscribing and publishing.
    pub bus: BusService,
}

/// Posts a transient chat entry summarising a session's settled discovery.
pub struct DiscoveryNotifierActor {
    bus: BusService,
}

impl Actor for DiscoveryNotifierActor {
    type Args = DiscoveryNotifierActorDeps;
    type Error = Infallible;

    async fn on_start(
        deps: Self::Args,
        actor_ref: ActorRef<Self>,
    ) -> Result<Self, Self::Error> {
        let bus = deps.bus;
        bus.actor_ref()
            .tell(Register(actor_ref.recipient::<SessionDiscoverySettled>()))
            .await
            .expect("register discovery notifier on bus");
        Ok(Self { bus })
    }
}

impl Message<SessionDiscoverySettled> for DiscoveryNotifierActor {
    type Reply = ();

    async fn handle(
        &mut self,
        event: SessionDiscoverySettled,
        _ctx: &mut Context<Self, Self::Reply>,
    ) {
        let summary = build_summary(&event);
        let push = PushChatEntry {
            session_id: event.session_id.clone(),
            entry: ChatEntry::transient(summary),
        };
        if let Err(e) = self.bus.actor_ref().tell(Publish(push)).await {
            tracing::warn!(err = ?e, "discovery-notifier failed to emit PushChatEntry");
        }
    }
}

/// Render a markdown summary of a settled discovery snapshot.
fn build_summary(event: &SessionDiscoverySettled) -> String {
    use std::fmt::Write as _;

    let snapshot = &event.snapshot;
    let mut out = String::new();
    let total = snapshot.skill_count + snapshot.prompt_count + snapshot.context_file_count;

    if total == 0 {
        out.push_str("No project resources found (no skills, prompts, or AGENTS.md).");
    } else {
        out.push_str("**Project resources discovered**\n");
        if snapshot.skill_count > 0 {
            let _ = writeln!(out, "- {} skill(s)", snapshot.skill_count);
        }
        if snapshot.prompt_count > 0 {
            let _ = writeln!(out, "- {} prompt(s)", snapshot.prompt_count);
        }
        if snapshot.context_file_count > 0 {
            let _ = writeln!(
                out,
                "- {} AGENTS.md / context file(s)",
                snapshot.context_file_count
            );
        }
    }

    let mut notes: Vec<String> = Vec::new();
    if let Some(reason) = &event.delayed {
        notes.push(reason.clone());
    }
    if let Some(err) = &snapshot.skill_error {
        notes.push(format!("skills scan error: {err}"));
    }
    if let Some(err) = &snapshot.prompt_error {
        notes.push(format!("prompts scan error: {err}"));
    }
    if let Some(err) = &snapshot.context_error {
        notes.push(format!("context-files scan error: {err}"));
    }

    if !notes.is_empty() {
        out.push('\n');
        for note in &notes {
            out.push_str(note.trim_end());
            out.push('\n');
        }
    }

    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unreachable, clippy::indexing_slicing, reason = "test code")]
    use super::*;
    use crate::feat::discovery_coordinator::DiscoverySnapshot;
    use crate::protocol::{ChatEntryKind, SessionId};
    use kameo::actor::Spawn;
    use kameo_actors::message_bus::MessageBus;
    use std::time::Duration;

    /// A recorder actor that collects messages of type M.
    pub struct Recorder<M> {
        recorded: Vec<M>,
    }

    pub struct GetRecorded;

    impl<M: Clone + Send + 'static> Actor for Recorder<M> {
        type Args = ();
        type Error = Infallible;

        async fn on_start(_: (), _: ActorRef<Self>) -> Result<Self, Self::Error> {
            Ok(Self { recorded: vec![] })
        }
    }

    impl<M: Clone + Send + 'static> Message<M> for Recorder<M> {
        type Reply = ();

        async fn handle(&mut self, msg: M, _ctx: &mut Context<Self, Self::Reply>) {
            self.recorded.push(msg);
        }
    }

    impl<M: Clone + Send + 'static> Message<GetRecorded> for Recorder<M> {
        type Reply = Vec<M>;

        async fn handle(
            &mut self,
            _: GetRecorded,
            _ctx: &mut Context<Self, Self::Reply>,
        ) -> Vec<M> {
            std::mem::take(&mut self.recorded)
        }
    }

    async fn setup() -> (
        ActorRef<DiscoveryNotifierActor>,
        ActorRef<Recorder<PushChatEntry>>,
        kameo::prelude::ActorRef<MessageBus>,
    ) {
        let bus_ref = kameo::actor::Spawn::spawn(MessageBus::new(
            kameo_actors::DeliveryStrategy::BestEffort,
        ));
        let bus_service = BusService::new(bus_ref.clone());

        let notifier = DiscoveryNotifierActor::spawn(DiscoveryNotifierActorDeps {
            bus: bus_service,
        });
        let recorder = Recorder::<PushChatEntry>::spawn(());
        bus_ref
            .tell(Register(recorder.clone().recipient::<PushChatEntry>()))
            .await
            .expect("register recorder");

        // Give the bus time to process registrations.
        tokio::time::sleep(Duration::from_millis(50)).await;

        (notifier, recorder, bus_ref)
    }

    #[tokio::test]
    async fn settled_event_posts_one_transient_chat_entry() {
        // Given a notifier and a recorder wired to the bus.
        let (_notifier, recorder, bus_ref) = setup().await;

        // When a SessionDiscoverySettled event with discovered resources is published.
        bus_ref
            .tell(Publish(SessionDiscoverySettled {
                session_id: SessionId::new(),
                snapshot: DiscoverySnapshot {
                    skill_count: 2,
                    prompt_count: 1,
                    context_file_count: 1,
                    ..Default::default()
                },
                delayed: None,
            }))
            .await
            .expect("publish settled event");

        // Then exactly one PushChatEntry was emitted with a transient entry.
        tokio::time::sleep(Duration::from_millis(100)).await;
        let recorded: Vec<PushChatEntry> = recorder.ask(GetRecorded).await.expect("get recorded");
        assert_eq!(recorded.len(), 1, "expected exactly one PushChatEntry");

        let entry = &recorded[0];
        assert!(
            matches!(entry.entry.kind, ChatEntryKind::Transient(_)),
            "entry should be transient"
        );
    }

    #[tokio::test]
    async fn empty_discovery_says_no_resources() {
        // Given a notifier and a recorder.
        let (_notifier, recorder, bus_ref) = setup().await;

        // When a settled event with empty discovery is published.
        bus_ref
            .tell(Publish(SessionDiscoverySettled {
                session_id: SessionId::new(),
                snapshot: DiscoverySnapshot::default(),
                delayed: None,
            }))
            .await
            .expect("publish settled event");

        // Then the message says no project resources found.
        tokio::time::sleep(Duration::from_millis(100)).await;
        let recorded: Vec<PushChatEntry> = recorder.ask(GetRecorded).await.expect("get recorded");
        let entry = &recorded[0];
        let ChatEntryKind::Transient(text) = &entry.entry.kind else {
            panic!("expected transient entry");
        };
        assert!(
            text.contains("No project resources found"),
            "expected no-resources message, got: {text}"
        );
    }

    #[tokio::test]
    async fn delayed_reason_surfaces_in_message() {
        // Given a notifier and a recorder.
        let (_notifier, recorder, bus_ref) = setup().await;

        // When a settled event carries a delayed reason.
        bus_ref
            .tell(Publish(SessionDiscoverySettled {
                session_id: SessionId::new(),
                snapshot: DiscoverySnapshot {
                    skill_count: 2,
                    ..Default::default()
                },
                delayed: Some("discovery delayed by context".to_owned()),
            }))
            .await
            .expect("publish settled event");

        // Then the reason is surfaced in the message.
        tokio::time::sleep(Duration::from_millis(100)).await;
        let recorded: Vec<PushChatEntry> = recorder.ask(GetRecorded).await.expect("get recorded");
        let entry = &recorded[0];
        let ChatEntryKind::Transient(text) = &entry.entry.kind else {
            panic!("expected transient entry");
        };
        assert!(
            text.contains("discovery delayed by context"),
            "expected delayed reason in message, got: {text}"
        );
    }

    #[tokio::test]
    async fn failed_scan_notes_error_in_message() {
        // Given a notifier and a recorder.
        let (_notifier, recorder, bus_ref) = setup().await;

        // When a settled event carries a skills scan error.
        bus_ref
            .tell(Publish(SessionDiscoverySettled {
                session_id: SessionId::new(),
                snapshot: DiscoverySnapshot {
                    skill_count: 0,
                    skill_error: Some("permission denied".to_owned()),
                    ..Default::default()
                },
                delayed: None,
            }))
            .await
            .expect("publish settled event");

        // Then the message notes the failure.
        tokio::time::sleep(Duration::from_millis(100)).await;
        let recorded: Vec<PushChatEntry> = recorder.ask(GetRecorded).await.expect("get recorded");
        let entry = &recorded[0];
        let ChatEntryKind::Transient(text) = &entry.entry.kind else {
            panic!("expected transient entry");
        };
        assert!(
            text.contains("skills scan error: permission denied"),
            "expected error note, got: {text}"
        );
    }
}
