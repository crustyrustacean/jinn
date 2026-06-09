//! Discovery notifier — turns the coalesced [`SessionDiscoverySettled`] event
//! into a visible transient chat-history entry for that session.
//!
//! The notifier subscribes to [`SessionDiscoverySettled`] (emitted by the
//! [`DiscoveryCoordinatorActor`](crate::feat::discovery_coordinator::coordinator_actor::DiscoveryCoordinatorActor))
//! and posts a markdown summary as a `Transient` chat entry via `PushChatEntry`.
//! One entry per settled event — it fires only on the coalesced signal, never
//! on every scan or render tick.

use crate::common::actor::{Actor, ActorContext, ActorEnvelope, NoDirectMsg};
use crate::feat::chat_input::protocol::command::PushChatEntry;
use crate::feat::discovery_coordinator::SessionDiscoverySettled;
use crate::protocol::{Command, Event};

/// Configuration for the discovery notifier. None today; reserved.
pub struct DiscoveryNotifierActorDeps;

/// Posts a transient chat entry summarising a session's settled discovery.
pub struct DiscoveryNotifierActor;

impl Actor for DiscoveryNotifierActor {
    type Message = NoDirectMsg;
    type Deps = DiscoveryNotifierActorDeps;

    fn activate(_deps: Self::Deps, ctx: &mut ActorContext) -> Self {
        ctx.subscribe_event::<SessionDiscoverySettled>();
        ctx.set_description("Posts a transient chat entry when a session's discovery settles");
        Self
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Event(event) => {
                if let Event::SessionDiscoverySettled(ref event) = event {
                    Self::on_settled(event, ctx);
                }
            }
            ActorEnvelope::Command(_) | ActorEnvelope::System(_) => {}
        }
    }
}

impl DiscoveryNotifierActor {
    /// Render the settled snapshot to a transient `PushChatEntry` command.
    fn on_settled(event: &SessionDiscoverySettled, ctx: &ActorContext) {
        let summary = build_summary(event);
        let push = Command::PushChatEntry(PushChatEntry {
            session_id: event.session_id.clone(),
            entry: crate::protocol::ChatEntry::transient(summary),
        });
        if let Err(e) = ctx.send_command(push) {
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
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use super::*;
    use crate::common::actor::{ActorContext, MessageSink, RecordingSink};
    use crate::feat::discovery_coordinator::DiscoverySnapshot;
    use crate::protocol::{ChatEntryKind, SessionId};
    use std::sync::Arc;

    /// Builds the notifier via `activate` with a recording sink wired into the
    /// context, returning the actor and its context.
    fn build() -> (DiscoveryNotifierActor, ActorContext, Arc<RecordingSink>) {
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new(
            "discovery-notifier-test",
            sink.clone() as Arc<dyn MessageSink>,
        );
        let actor = DiscoveryNotifierActor::activate(DiscoveryNotifierActorDeps, &mut ctx);
        (actor, ctx, sink)
    }

    async fn run_settled(actor: &mut DiscoveryNotifierActor, ctx: &ActorContext, event: Event) {
        actor.handle(ActorEnvelope::Event(event), ctx).await;
    }

    #[tokio::test]
    async fn settled_event_posts_one_transient_chat_entry() {
        // Given a notifier actor.
        let (mut actor, ctx, sink) = build();

        // When a SessionDiscoverySettled event with discovered resources is handled.
        run_settled(
            &mut actor,
            &ctx,
            Event::SessionDiscoverySettled(SessionDiscoverySettled {
                session_id: SessionId::new(),
                snapshot: DiscoverySnapshot {
                    skill_count: 2,
                    prompt_count: 1,
                    context_file_count: 1,
                    ..Default::default()
                },
                delayed: None,
            }),
        )
        .await;

        // Then exactly one PushChatEntry command was emitted.
        let commands: Vec<_> = sink
            .commands()
            .into_iter()
            .filter(|c| matches!(c, Command::PushChatEntry(_)))
            .collect();
        assert_eq!(commands.len(), 1, "expected exactly one PushChatEntry");
        if let Command::PushChatEntry(PushChatEntry { entry, .. }) = &commands[0] {
            assert!(
                matches!(entry.kind, ChatEntryKind::Transient(_)),
                "entry should be transient"
            );
        } else {
            panic!("expected PushChatEntry");
        }
    }

    #[tokio::test]
    async fn empty_discovery_says_no_resources() {
        // Given a notifier.
        let (mut actor, ctx, sink) = build();

        // When a settled event with empty discovery is handled.
        run_settled(
            &mut actor,
            &ctx,
            Event::SessionDiscoverySettled(SessionDiscoverySettled {
                session_id: SessionId::new(),
                snapshot: DiscoverySnapshot::default(),
                delayed: None,
            }),
        )
        .await;

        // Then the message says no project resources found.
        let Command::PushChatEntry(PushChatEntry { entry, .. }) = &sink.commands()[0] else {
            panic!("expected PushChatEntry");
        };
        let ChatEntryKind::Transient(text) = &entry.kind else {
            panic!("expected transient entry");
        };
        assert!(
            text.contains("No project resources found"),
            "expected no-resources message, got: {text}"
        );
    }

    #[tokio::test]
    async fn delayed_reason_surfaces_in_message() {
        // Given a notifier.
        let (mut actor, ctx, sink) = build();

        // When a settled event carries a delayed reason.
        run_settled(
            &mut actor,
            &ctx,
            Event::SessionDiscoverySettled(SessionDiscoverySettled {
                session_id: SessionId::new(),
                snapshot: DiscoverySnapshot {
                    skill_count: 2,
                    ..Default::default()
                },
                delayed: Some("discovery delayed by context".to_owned()),
            }),
        )
        .await;

        // Then the reason is surfaced in the message.
        let Command::PushChatEntry(PushChatEntry { entry, .. }) = &sink.commands()[0] else {
            panic!("expected PushChatEntry");
        };
        let ChatEntryKind::Transient(text) = &entry.kind else {
            panic!("expected transient entry");
        };
        assert!(
            text.contains("discovery delayed by context"),
            "expected delayed reason in message, got: {text}"
        );
    }

    #[tokio::test]
    async fn failed_scan_notes_error_in_message() {
        // Given a notifier.
        let (mut actor, ctx, sink) = build();

        // When a settled event carries a skills scan error.
        run_settled(
            &mut actor,
            &ctx,
            Event::SessionDiscoverySettled(SessionDiscoverySettled {
                session_id: SessionId::new(),
                snapshot: DiscoverySnapshot {
                    skill_count: 0,
                    skill_error: Some("permission denied".to_owned()),
                    ..Default::default()
                },
                delayed: None,
            }),
        )
        .await;

        // Then the message notes the failure.
        let Command::PushChatEntry(PushChatEntry { entry, .. }) = &sink.commands()[0] else {
            panic!("expected PushChatEntry");
        };
        let ChatEntryKind::Transient(text) = &entry.kind else {
            panic!("expected transient entry");
        };
        assert!(
            text.contains("skills scan error: permission denied"),
            "expected error note, got: {text}"
        );
    }
}
