//! Compaction trigger actor - handles `/compact` and `/compact-all` commands.
//!
//! Receives `TriggerCompaction` commands, runs the compaction worker,
//! and submits mutations via `SubmitHistoryMutations`.
//! Pushes system messages for user feedback (queued, skipped, failed).

use crate::common::actor::{Actor, ActorContext, ActorEnvelope, NoDirectMsg};
use crate::feat::chat_input::protocol::command::PushChatEntry;
use crate::feat::compaction_worker::worker::CompactionWorker;
use crate::feat::session::chat_entry::ChatEntry;
use crate::feat::session::protocol::submit_history_mutations::SubmitHistoryMutations;
use crate::feat::session::protocol::trigger_compaction::TriggerCompaction;
use crate::protocol::Command;

/// Actor that handles manual compaction triggers.
///
/// Subscribes to `TriggerCompaction` commands (from `/compact` and `/compact-all`).
/// Runs the compaction worker and submits mutations.
pub struct CompactionTriggerActor {
    worker: CompactionWorker,
}

/// Dependencies for [`CompactionTriggerActor`].
pub struct CompactionTriggerActorDeps {
    /// The compaction worker.
    pub worker: CompactionWorker,
}

impl Actor for CompactionTriggerActor {
    type Message = NoDirectMsg;
    type Deps = CompactionTriggerActorDeps;

    fn activate(deps: Self::Deps, ctx: &mut ActorContext) -> Self {
        ctx.set_description("compaction-trigger");
        ctx.subscribe_command::<TriggerCompaction>();

        Self {
            worker: deps.worker,
        }
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Command(Command::TriggerCompaction(ref payload)) => {
                self.handle_trigger_compaction(payload, ctx).await;
            }
            ActorEnvelope::Command(_)
            | ActorEnvelope::Event(_)
            | ActorEnvelope::System(_)
            | ActorEnvelope::Direct(_) => {}
        }
    }
}

impl CompactionTriggerActor {
    async fn handle_trigger_compaction(&self, payload: &TriggerCompaction, ctx: &ActorContext) {
        tracing::info!(
            session_id = %payload.session_id,
            compact_all = payload.compact_all,
            "manual compaction triggered"
        );

        // Always push immediate "queued" feedback.
        let _ = ctx.send_command(Command::PushChatEntry(PushChatEntry {
            session_id: payload.session_id.clone(),
            entry: ChatEntry::system("⏳ Compacting context..."),
        }));

        let trigger = crate::feat::compaction_worker::worker::CompactionTrigger {
            session_id: payload.session_id.clone(),
            compact_all: payload.compact_all,
        };

        match self.worker.evaluate_for_session(&trigger).await {
            Ok(mutations) if !mutations.is_empty() => {
                tracing::info!(
                    session_id = %payload.session_id,
                    count = mutations.len(),
                    "compaction trigger produced mutations"
                );

                let cmd = Command::SubmitHistoryMutations(SubmitHistoryMutations {
                    session_id: payload.session_id.clone(),
                    mutations,
                });

                if let Err(e) = ctx.send_command(cmd) {
                    tracing::warn!(
                        err = ?e,
                        "failed to submit compaction mutations"
                    );
                }
            }
            Ok(_) => {
                // Empty mutations - nothing to compact.
                let reserve = self.worker.state.read()
                    .frontend
                    .preferences
                    .compaction
                    .reserve_tokens;
                let msg = format!(
                    "⚠ Compaction skipped: recent conversation fits within reserve ({reserve} tokens)."
                );
                tracing::info!(
                    session_id = %payload.session_id,
                    "compaction produced no mutations (nothing to compact)"
                );
                let _ = ctx.send_command(Command::PushChatEntry(PushChatEntry {
                    session_id: payload.session_id.clone(),
                    entry: ChatEntry::system(&msg),
                }));
            }
            Err(e) => {
                tracing::error!(
                    session_id = %payload.session_id,
                    error = %e,
                    "compaction failed"
                );
                let msg = format!("⚠ Compaction failed: {e}");
                let _ = ctx.send_command(Command::PushChatEntry(PushChatEntry {
                    session_id: payload.session_id.clone(),
                    entry: ChatEntry::system(&msg),
                }));
            }
        }
    }
}
