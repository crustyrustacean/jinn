//! Todo auto-steer worker — periodic task-list reminders.
//!
//! Detects when an agent has gone too long without touching the todo list and
//! injects a `User` reminder entry at the tail of history nudging it to update
//! the list. This counteracts a common failure mode: agents drift through long
//! stretches of work without updating the task list, then spam `todo_*` calls
//! at the end to reconcile.
//!
//! # Cadence
//!
//! Analysis of real session data shows exemplary agents check the todo list on
//! a median cadence of ~9 history entries (p90 ≈ 30). The default `threshold`
//! (100) is deliberately lenient: it tolerates common multi-step recovery (an
//! agent getting unstuck) without nagging, while still catching genuine drift
//! well past exemplary behavior.
//!
//! # Anchor model
//!
//! The worker scans history for the **most recent** of:
//! - a `todo_*` ToolCall, or
//! - a previously-injected auto-steer reminder (recognized by [`STEER_SENTINEL`]).
//!
//! If ≥ `threshold` entries have elapsed since that anchor, it inserts one
//! reminder at the tail. Recognizing its own reminders as anchors both
//! implements the "re-nag every `threshold` entries" cadence and prevents an
//! infinite loop (an inserted entry appends history → re-triggers the worker).
//!
//! # Pending-id guard
//!
//! The `InsertEntry` mutation is deferred — queued in `pending_mutations`
//! until the next tool-loop boundary, not applied immediately. Between emit
//! and application, additional `HistoryAppended` events fire and re-snapshot
//! history that does **not** yet contain the reminder. To prevent a duplicate
//! emit on those snapshots, the worker pre-generates the reminder's entry ID
//! and stashes it (`pending_steer_id`). Subsequent snapshots suppress
//! re-evaluation until that ID appears in history (reminder landed), then
//! clear the guard. This is the same TOCTOU guard compaction uses
//! (`pending_compaction_id`).
//!
//! # Gate
//!
//! Never emits before the session has at least one `todo_*` call, so casual
//! Q&A sessions without an initialized task list stay quiet.

use crate::feat::history_worker::worker_trait::HistoryWorker;
use crate::feat::session::chat_entry::{ChatEntry, ChatEntryId, ChatEntryKind};
use crate::feat::session::history_mutation::HistoryMutation;
use crate::protocol::SessionId;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

/// Default enabled state for todo auto-steer.
const DEFAULT_TODO_STEER_ENABLED: bool = true;

/// Default reminder threshold for todo auto-steer.
///
/// Number of history entries that must elapse after the most recent `todo_*`
/// call (or prior reminder) before a new reminder is injected. Tuned for a
/// lenient cadence that tolerates common multi-step recovery without nagging,
/// while still catching genuine drift.
const DEFAULT_TODO_STEER_THRESHOLD: usize = 100;

/// Stable, recognizable sentinel prefixing every auto-steer reminder's entry
/// text so the worker can identify its own previously-injected reminders when
/// scanning history on subsequent evaluations.
const STEER_SENTINEL: &str = "[auto-steer]";

/// Todo auto-steer configuration.
///
/// Serialized as `[todo_auto_steer]` in `jinn.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TodoAutoSteerConfig {
    /// Whether the todo auto-steer worker is active.
    /// Default: `true`.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Number of history entries that must elapse after the most recent
    /// `todo_*` call (or prior reminder) before a new reminder is injected.
    /// Default: `100`.
    #[serde(default = "default_threshold")]
    pub threshold: usize,
}

fn default_enabled() -> bool {
    DEFAULT_TODO_STEER_ENABLED
}

fn default_threshold() -> usize {
    DEFAULT_TODO_STEER_THRESHOLD
}

impl Default for TodoAutoSteerConfig {
    fn default() -> Self {
        Self {
            enabled: DEFAULT_TODO_STEER_ENABLED,
            threshold: DEFAULT_TODO_STEER_THRESHOLD,
        }
    }
}

/// Returns true if a tool name belongs to the todo tool group.
fn is_todo_tool(name: &str) -> bool {
    name.starts_with("todo_")
}

/// Returns true if a chat entry is an auto-steer reminder previously injected
/// by this worker (identified by the [`STEER_SENTINEL`] prefix on its text).
fn is_steer_reminder(entry: &ChatEntry) -> bool {
    match &entry.kind {
        ChatEntryKind::User { expanded, .. } => expanded.starts_with(STEER_SENTINEL),
        _ => false,
    }
}

/// A background worker that injects periodic todo-list reminders.
///
/// See the [module docs](self) for the cadence, anchor model, and gate.
#[derive(Clone)]
pub struct TodoAutoSteerWorker {
    /// Worker configuration.
    pub config: TodoAutoSteerConfig,
    /// The `ChatEntryId` of the most recently emitted reminder whose
    /// `InsertEntry` mutation has not yet been observed in a snapshot.
    ///
    /// Set when a reminder is emitted, cleared once a subsequent snapshot
    /// contains that entry. While set, the worker suppresses re-evaluation
    /// so the deferred mutation has time to apply — closing the
    /// scan→application TOCTOU that otherwise produces duplicate reminders.
    /// `None` means no reminder is in flight.
    pub pending_steer_id: Arc<Mutex<Option<ChatEntryId>>>,
}

#[async_trait::async_trait]
impl HistoryWorker for TodoAutoSteerWorker {
    #[expect(
        clippy::unnecessary_literal_bound,
        reason = "lifetime elision makes bound redundant"
    )]
    fn name(&self) -> &str {
        "auto-steer-todo"
    }

    async fn evaluate(
        &self,
        _session_id: &SessionId,
        history: Arc<[ChatEntry]>,
    ) -> Vec<HistoryMutation> {
        if !self.config.enabled {
            return Vec::new();
        }

        // GUARD: if a reminder was emitted on a previous snapshot but its
        // deferred InsertEntry mutation has not yet appeared in history,
        // suppress re-evaluation to avoid emitting a duplicate. Once the
        // entry is observed, clear the guard and proceed normally. The
        // mutex is never held across the rest of evaluate (clone-and-drop).
        let pending = self.pending_steer_id.lock().unwrap().clone();
        if let Some(pending_id) = pending {
            let landed = history.iter().any(|entry| entry.id == pending_id);
            if landed {
                *self.pending_steer_id.lock().unwrap() = None;
                tracing::debug!(
                    "auto-steer reminder {:?} found in snapshot, clearing guard",
                    pending_id
                );
            } else {
                tracing::info!("auto-steer reminder in flight, skipping snapshot");
                return Vec::new();
            }
        }

        let (mutations, emitted_id) = build_steer_mutations(&history, self.config.threshold);

        // STASH: record the pre-generated ID so the next snapshot can detect
        // whether this reminder has landed yet.
        if let Some(id) = emitted_id {
            *self.pending_steer_id.lock().unwrap() = Some(id);
        }

        mutations
    }
}

/// Build the (at most one) reminder mutation for the given history snapshot.
///
/// Returns an empty vec if gated or below threshold. When a reminder is
/// emitted, also returns the pre-generated `ChatEntryId` of its entry so the
/// caller can stash it as the pending-id guard.
///
/// The entry ID is pre-generated (not minted inside `ChatEntry::user_expanded`)
/// so the worker can track this exact entry until it appears in a snapshot.
fn build_steer_mutations(
    history: &[ChatEntry],
    threshold: usize,
) -> (Vec<HistoryMutation>, Option<ChatEntryId>) {
    // GATE: never emit until the session has at least one todo_* ToolCall.
    // Scanned independently of the anchor so correctness does not depend on
    // the invariant that entries are never removed from the snapshot.
    let has_todo = history.iter().any(|entry| {
        matches!(
            &entry.kind,
            ChatEntryKind::ToolCall { name, .. } if is_todo_tool(name)
        )
    });
    if !has_todo {
        return (Vec::new(), None);
    }

    // Iterate from the end (most recent) toward the start to find the most
    // recent anchor: a todo_ ToolCall or a prior steer reminder.
    let anchor_index = history.iter().enumerate().rev().find_map(|(i, entry)| {
        let is_todo_call = matches!(
            &entry.kind,
            ChatEntryKind::ToolCall { name, .. } if is_todo_tool(name)
        );
        (is_todo_call || is_steer_reminder(entry)).then_some(i)
    });

    // has_todo is true, so an anchor (the todo_ call) always exists here.
    let Some(anchor_index) = anchor_index else {
        return (Vec::new(), None);
    };

    let elapsed = history.len().saturating_sub(1).saturating_sub(anchor_index);
    if elapsed < threshold {
        return (Vec::new(), None);
    }

    let Some(last) = history.last() else {
        return (Vec::new(), None);
    };

    let text = format!(
        "{STEER_SENTINEL} You haven't updated the task list in {elapsed} entries. \
         Use a `todo_*` tool to review and update your task list."
    );

    // Pre-generate the entry ID so the worker can track it as pending until
    // the deferred InsertEntry mutation lands in a snapshot. `new_with_kind`
    // (not `user_expanded`) is used because the latter mints its own ID.
    let id = ChatEntryId::new();
    let entry = ChatEntry::new_with_kind(
        id.clone(),
        crate::protocol::EntryTiming::instant_now(),
        ChatEntryKind::User {
            display: text.clone(),
            expanded: text,
            attachments: Vec::new(),
        },
        None,
    );

    (
        vec![HistoryMutation::InsertEntry {
            after_entry_id: Some(last.id.clone()),
            entry,
        }],
        Some(id),
    )
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
    use crate::feat::session::chat_entry::ChatEntry;
    use crate::feat::session::history_mutation::HistoryMutation;
    use crate::protocol::SessionId;
    use std::sync::{Arc, Mutex};

    fn sid() -> SessionId {
        SessionId::new()
    }

    /// One todo ToolCall entry.
    fn todo_call(name: &str) -> ChatEntry {
        ChatEntry::tool_call(format!("call-{name}"), name, "{}")
    }

    /// One non-todo entry (assistant filler).
    fn filler() -> ChatEntry {
        ChatEntry::assistant("working")
    }

    /// N filler entries.
    fn fillers(n: usize) -> Vec<ChatEntry> {
        std::iter::repeat_with(filler).take(n).collect()
    }

    /// A prior auto-steer reminder entry.
    fn reminder(n: usize) -> ChatEntry {
        ChatEntry::user_expanded(
            format!("{STEER_SENTINEL} reminder {n}"),
            format!("{STEER_SENTINEL} reminder {n}"),
        )
    }

    /// A prior auto-steer reminder entry with a specific (known) ID, for guard
    /// tests that need the pending ID to appear in history.
    fn reminder_with_id(id: ChatEntryId) -> ChatEntry {
        ChatEntry::new_with_kind(
            id,
            crate::protocol::EntryTiming::instant_now(),
            ChatEntryKind::User {
                display: format!("{STEER_SENTINEL} reminder"),
                expanded: format!("{STEER_SENTINEL} reminder"),
                attachments: Vec::new(),
            },
            None,
        )
    }

    async fn run(history: &[ChatEntry], config: TodoAutoSteerConfig) -> Vec<HistoryMutation> {
        run_with_pending(history, config, None).await
    }

    /// Like `run`, but seeds the worker's pending-id guard with `pending`.
    async fn run_with_pending(
        history: &[ChatEntry],
        config: TodoAutoSteerConfig,
        pending: Option<ChatEntryId>,
    ) -> Vec<HistoryMutation> {
        let worker = TodoAutoSteerWorker {
            config,
            pending_steer_id: Arc::new(Mutex::new(pending)),
        };
        worker.evaluate(&sid(), Arc::from(history)).await
    }

    /// A worker pre-seeded with a pending ID, returned alongside the ID so a
    /// test can assert on it after evaluation.
    fn worker_with_pending(
        config: TodoAutoSteerConfig,
        pending: Option<ChatEntryId>,
    ) -> (TodoAutoSteerWorker, Option<ChatEntryId>) {
        (
            TodoAutoSteerWorker {
                config,
                pending_steer_id: Arc::new(Mutex::new(pending.clone())),
            },
            pending,
        )
    }

    #[tokio::test]
    async fn empty_history_produces_no_mutations() {
        // Given an empty history with no todo_ calls.
        // When evaluating.
        let mutations = run(&[], TodoAutoSteerConfig::default()).await;

        // Then no mutations are produced.
        assert!(mutations.is_empty());
    }

    #[tokio::test]
    async fn no_todo_calls_below_threshold_produces_no_mutations() {
        // Given a history with no todo_ calls but fewer than threshold entries.
        let history: Vec<ChatEntry> = fillers(10);

        // When evaluating.
        let mutations = run(&history, TodoAutoSteerConfig::default()).await;

        // Then no mutations are produced (gated: no todo_ call exists).
        assert!(mutations.is_empty());
    }

    #[tokio::test]
    async fn no_todo_calls_above_threshold_produces_no_mutations() {
        // Given a history with no todo_ calls but more than threshold entries.
        let history: Vec<ChatEntry> = fillers(50);

        // When evaluating.
        let mutations = run(&history, TodoAutoSteerConfig::default()).await;

        // Then no mutations are produced (gated: no todo_ call exists).
        assert!(mutations.is_empty());
    }

    #[tokio::test]
    async fn reminder_without_any_todo_call_is_gated() {
        // Given a history with a prior reminder but NO todo_* ToolCall anywhere.
        let mut history = vec![reminder(0)];
        history.extend(fillers(40));

        // When evaluating.
        let mutations = run(&history, TodoAutoSteerConfig::default()).await;

        // Then no mutations are produced (gate: no todo_* call has ever occurred).
        assert!(mutations.is_empty());
    }

    #[tokio::test]
    async fn todo_call_below_threshold_produces_no_mutations() {
        // Given a history with a todo_ call, elapsed entries below threshold.
        let mut history = vec![todo_call("todo_add_task")];
        history.extend(fillers(10));

        // When evaluating.
        let mutations = run(&history, TodoAutoSteerConfig::default()).await;

        // Then no mutations are produced.
        assert!(mutations.is_empty());
    }

    #[tokio::test]
    async fn todo_call_at_threshold_produces_one_tail_insert() {
        // Given a history with a todo_ call and exactly threshold entries after.
        let mut history = vec![todo_call("todo_add_task")];
        history.extend(fillers(30));

        // When evaluating with a small explicit threshold matching the filler count.
        let mutations = run(
            &history,
            TodoAutoSteerConfig {
                enabled: true,
                threshold: 30,
            },
        )
        .await;

        // Then exactly one InsertEntry mutation is produced at the tail.
        assert_eq!(mutations.len(), 1);
        match &mutations[0] {
            HistoryMutation::InsertEntry { after_entry_id, .. } => {
                assert_eq!(
                    *after_entry_id.as_ref().unwrap(),
                    history.last().unwrap().id
                );
            }
            other => panic!("expected InsertEntry, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn todo_call_within_threshold_suppresses_reminder() {
        // Given a history with two todo_ calls, the second within threshold of end.
        let mut history = fillers(40);
        history.insert(0, todo_call("todo_add_task"));
        history.push(todo_call("todo_complete_task"));
        history.extend(fillers(5));

        // When evaluating.
        let mutations = run(&history, TodoAutoSteerConfig::default()).await;

        // Then no mutations are produced (recent todo_ call resets anchor).
        assert!(mutations.is_empty());
    }

    #[tokio::test]
    async fn existing_reminder_at_threshold_produces_another_insert() {
        // Given a history with a prior reminder and threshold entries after it.
        let mut history = vec![todo_call("todo_add_task"), reminder(0)];
        history.extend(fillers(30));

        // When evaluating with a small explicit threshold matching the filler count.
        let mutations = run(
            &history,
            TodoAutoSteerConfig {
                enabled: true,
                threshold: 30,
            },
        )
        .await;

        // Then exactly one InsertEntry mutation is produced (re-arm).
        assert_eq!(mutations.len(), 1);
        assert!(matches!(mutations[0], HistoryMutation::InsertEntry { .. }));
    }

    #[tokio::test]
    async fn existing_reminder_below_threshold_produces_no_mutations() {
        // Given a history with a prior reminder and fewer than threshold entries after.
        let mut history = vec![todo_call("todo_add_task"), reminder(0)];
        history.extend(fillers(10));

        // When evaluating.
        let mutations = run(&history, TodoAutoSteerConfig::default()).await;

        // Then no mutations are produced (reminder is the anchor, below threshold).
        assert!(mutations.is_empty());
    }

    #[tokio::test]
    async fn disabled_produces_no_mutations_regardless_of_history() {
        // Given a history that would trigger a reminder.
        let mut history = vec![todo_call("todo_add_task")];
        history.extend(fillers(50));

        // When evaluating with the worker disabled.
        let config = TodoAutoSteerConfig {
            enabled: false,
            threshold: 100,
        };
        let mutations = run(&history, config).await;

        // Then no mutations are produced.
        assert!(mutations.is_empty());
    }

    #[tokio::test]
    async fn read_only_todo_call_resets_anchor() {
        // Given a history where only a read-only todo_ call exists within threshold.
        let mut history = vec![todo_call("todo_get_task_list")];
        history.extend(fillers(5));

        // When evaluating.
        let mutations = run(&history, TodoAutoSteerConfig::default()).await;

        // Then no mutations are produced (read-only call counts as engagement).
        assert!(mutations.is_empty());
    }

    #[tokio::test]
    async fn produced_entry_is_user_kind_at_tail() {
        // Given a history with a todo_ call and threshold entries after.
        let mut history = vec![todo_call("todo_add_task")];
        history.extend(fillers(30));
        let last_id = history.last().unwrap().id.clone();

        // When evaluating with a small explicit threshold matching the filler count.
        let mutations = run(
            &history,
            TodoAutoSteerConfig {
                enabled: true,
                threshold: 30,
            },
        )
        .await;

        // Then the mutation inserts a User entry at the tail.
        match &mutations[0] {
            HistoryMutation::InsertEntry {
                after_entry_id,
                entry,
            } => {
                assert_eq!(*after_entry_id.as_ref().unwrap(), last_id);
                assert!(
                    matches!(&entry.kind, ChatEntryKind::User { .. }),
                    "reminder must be a User entry"
                );
                assert!(
                    entry
                        .prompt_text()
                        .unwrap_or_default()
                        .starts_with(STEER_SENTINEL),
                    "reminder text must start with the sentinel"
                );
            }
            other => panic!("expected InsertEntry, got {other:?}"),
        }
    }

    // ── Pending-id guard tests ──

    #[tokio::test]
    async fn pending_id_not_in_history_skips_evaluation() {
        // Given a worker with a pending reminder whose ID is not in history,
        // and a history at threshold that would otherwise emit.
        let pending_id = ChatEntryId::new();
        let mut history = vec![todo_call("todo_add_task")];
        history.extend(fillers(30));

        // When evaluating.
        let mutations = run_with_pending(
            &history,
            TodoAutoSteerConfig {
                enabled: true,
                threshold: 30,
            },
            Some(pending_id.clone()),
        )
        .await;

        // Then no mutations are produced (guard suppresses re-evaluation).
        assert!(mutations.is_empty());
    }

    #[tokio::test]
    async fn pending_id_not_in_history_remains_set() {
        // Given a worker with a pending reminder whose ID is not in history.
        let pending_id = ChatEntryId::new();
        let mut history = vec![todo_call("todo_add_task")];
        history.extend(fillers(30));

        // When evaluating with a worker we can inspect afterwards.
        let (worker, _) = worker_with_pending(
            TodoAutoSteerConfig {
                enabled: true,
                threshold: 30,
            },
            Some(pending_id.clone()),
        );
        let mutations = worker.evaluate(&sid(), Arc::from(&history[..])).await;

        // Then no mutations are produced.
        assert!(mutations.is_empty());
        // And the pending ID remains set (not cleared) since it wasn't found.
        let after = worker.pending_steer_id.lock().unwrap().clone();
        assert_eq!(after, Some(pending_id));
    }

    #[tokio::test]
    async fn pending_id_found_in_history_clears_and_proceeds() {
        // Given a worker with a pending reminder whose ID IS in history,
        // and a history at threshold for a fresh emit.
        let pending_id = ChatEntryId::new();
        let mut history = vec![
            todo_call("todo_add_task"),
            reminder_with_id(pending_id.clone()),
        ];
        history.extend(fillers(30));

        // When evaluating with a worker we can inspect afterwards.
        let (worker, _) = worker_with_pending(
            TodoAutoSteerConfig {
                enabled: true,
                threshold: 30,
            },
            Some(pending_id.clone()),
        );
        let mutations = worker.evaluate(&sid(), Arc::from(&history[..])).await;

        // Then a fresh mutation is produced (the pending reminder landed,
        // so the guard was cleared and evaluation proceeded).
        assert_eq!(mutations.len(), 1);
        // And the original pending ID is no longer stashed — it was cleared
        // and replaced by the fresh emit's new ID.
        let after = worker.pending_steer_id.lock().unwrap().clone();
        assert_ne!(
            after.as_ref(),
            Some(&pending_id),
            "original pending ID should be cleared after landing"
        );
    }

    #[tokio::test]
    async fn emit_sets_pending_id() {
        // Given a fresh worker (no pending) and a history at threshold.
        let mut history = vec![todo_call("todo_add_task")];
        history.extend(fillers(30));

        // When evaluating with a worker we can inspect afterwards.
        let (worker, _) = worker_with_pending(
            TodoAutoSteerConfig {
                enabled: true,
                threshold: 30,
            },
            None,
        );
        let mutations = worker.evaluate(&sid(), Arc::from(&history[..])).await;

        // Then exactly one mutation is emitted.
        assert_eq!(mutations.len(), 1);
        // And the worker's pending_steer_id is now set.
        let pending = worker.pending_steer_id.lock().unwrap().clone();
        assert!(pending.is_some(), "pending should be set after emit");
    }

    #[tokio::test]
    async fn emitted_entry_carries_stashed_id() {
        // Given a fresh worker and a history at threshold.
        let mut history = vec![todo_call("todo_add_task")];
        history.extend(fillers(30));

        // When evaluating.
        let (worker, _) = worker_with_pending(
            TodoAutoSteerConfig {
                enabled: true,
                threshold: 30,
            },
            None,
        );
        let mutations = worker.evaluate(&sid(), Arc::from(&history[..])).await;

        // Then the emitted entry's ID equals the stashed pending ID.
        let stashed = worker.pending_steer_id.lock().unwrap().clone();
        let emitted_id = match &mutations[0] {
            HistoryMutation::InsertEntry { entry, .. } => entry.id.clone(),
            other => panic!("expected InsertEntry, got {other:?}"),
        };
        assert_eq!(stashed, Some(emitted_id));
    }

    #[tokio::test]
    async fn disabled_does_not_touch_guard() {
        // Given a disabled worker with a pending ID set.
        let pending_id = ChatEntryId::new();
        let mut history = vec![todo_call("todo_add_task")];
        history.extend(fillers(30));

        // When evaluating.
        let (worker, _) = worker_with_pending(
            TodoAutoSteerConfig {
                enabled: false,
                threshold: 30,
            },
            Some(pending_id.clone()),
        );
        let mutations = worker.evaluate(&sid(), Arc::from(&history[..])).await;

        // Then no mutations are produced (disabled wins).
        assert!(mutations.is_empty());
        // And the pending ID is untouched (not inspected, not cleared).
        let after = worker.pending_steer_id.lock().unwrap().clone();
        assert_eq!(after, Some(pending_id));
    }
}
