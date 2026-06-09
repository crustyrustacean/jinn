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

use crate::common::actor_deps::{ActorDeps, BusPublish};
use crate::feat::chat_input::protocol::command::PushChatEntry;
use crate::feat::discovery_coordinator::SessionDiscoverySettled;
use crate::protocol::ChatEntry;

/// Dependencies injected at startup.
pub struct DiscoveryNotifierActorDeps {
    /// Universal actor dependencies (bus, services, etc.).
    pub deps: ActorDeps,
}

/// Posts a transient chat entry summarising a session's settled discovery.
pub struct DiscoveryNotifierActor {
    deps: ActorDeps,
}

impl Actor for DiscoveryNotifierActor {
    type Args = DiscoveryNotifierActorDeps;
    type Error = Infallible;

    async fn on_start(args: Self::Args, actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        args.deps
            .subscribe(actor_ref.recipient::<SessionDiscoverySettled>())
            .await;
        Ok(Self { deps: args.deps })
    }
}

impl Message<SessionDiscoverySettled> for DiscoveryNotifierActor {
    type Reply = ();

    async fn handle(
        &mut self,
        msg: SessionDiscoverySettled,
        _ctx: &mut Context<Self, Self::Reply>,
    ) {
        let summary = build_summary(&msg);
        let push = PushChatEntry {
            session_id: msg.session_id.clone(),
            entry: ChatEntry::transient(summary),
        };
        self.publish(push).await;
    }
}

impl BusPublish for DiscoveryNotifierActor {
    fn bus(&self) -> &crate::common::services::bus_service::BusService {
        self.deps.bus()
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
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]

    use std::time::Duration;

    use super::*;
    use crate::common::bus::test_harness::{TestHarness, await_recorded};
    use crate::feat::discovery_coordinator::DiscoverySnapshot;
    use crate::protocol::{ChatEntryKind, SessionId};

    #[tokio::test]
    async fn settled_event_posts_one_transient_chat_entry() {
        // Given a notifier and a recorder wired to the bus.
        let harness = TestHarness::new().await;
        let _notifier = harness
            .spawn_actor::<DiscoveryNotifierActor>(DiscoveryNotifierActorDeps {
                deps: harness.actor_deps(),
            })
            .await;
        let recorder = harness.spawn_recorder::<PushChatEntry>().await;

        // When a SessionDiscoverySettled event with discovered resources is published.
        harness
            .publish(SessionDiscoverySettled {
                session_id: SessionId::new(),
                snapshot: DiscoverySnapshot {
                    skill_count: 2,
                    prompt_count: 1,
                    context_file_count: 1,
                    ..Default::default()
                },
                delayed: None,
            })
            .await;

        // Then exactly one PushChatEntry was emitted with a transient entry.
        let recorded = await_recorded(&recorder, 1, Duration::from_millis(500)).await;
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
        let harness = TestHarness::new().await;
        let _notifier = harness
            .spawn_actor::<DiscoveryNotifierActor>(DiscoveryNotifierActorDeps {
                deps: harness.actor_deps(),
            })
            .await;
        let recorder = harness.spawn_recorder::<PushChatEntry>().await;

        // When a settled event with empty discovery is published.
        harness
            .publish(SessionDiscoverySettled {
                session_id: SessionId::new(),
                snapshot: DiscoverySnapshot::default(),
                delayed: None,
            })
            .await;

        // Then the message says no project resources found.
        let recorded = await_recorded(&recorder, 1, Duration::from_millis(500)).await;
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
        let harness = TestHarness::new().await;
        let _notifier = harness
            .spawn_actor::<DiscoveryNotifierActor>(DiscoveryNotifierActorDeps {
                deps: harness.actor_deps(),
            })
            .await;
        let recorder = harness.spawn_recorder::<PushChatEntry>().await;

        // When a settled event carries a delayed reason.
        harness
            .publish(SessionDiscoverySettled {
                session_id: SessionId::new(),
                snapshot: DiscoverySnapshot {
                    skill_count: 2,
                    ..Default::default()
                },
                delayed: Some("discovery delayed by context".to_owned()),
            })
            .await;

        // Then the reason is surfaced in the message.
        let recorded = await_recorded(&recorder, 1, Duration::from_millis(500)).await;
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
        let harness = TestHarness::new().await;
        let _notifier = harness
            .spawn_actor::<DiscoveryNotifierActor>(DiscoveryNotifierActorDeps {
                deps: harness.actor_deps(),
            })
            .await;
        let recorder = harness.spawn_recorder::<PushChatEntry>().await;

        // When a settled event carries a skills scan error.
        harness
            .publish(SessionDiscoverySettled {
                session_id: SessionId::new(),
                snapshot: DiscoverySnapshot {
                    skill_count: 0,
                    skill_error: Some("permission denied".to_owned()),
                    ..Default::default()
                },
                delayed: None,
            })
            .await;

        // Then the message notes the failure.
        let recorded = await_recorded(&recorder, 1, Duration::from_millis(500)).await;
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
