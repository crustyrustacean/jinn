#![allow(clippy::expect_used, clippy::indexing_slicing)]

//! Tests for the compaction actor.

use crate::feat::compaction_actor::serializer::serialize_entries_for_compaction;
use crate::feat::session::chat_entry::{ChatEntry, ChatEntryKind};
use crate::feat::session::tool_result_status::ToolResultStatus;
use crate::protocol::PinPosition;

use crate::common::actor::Actor;
use crate::common::actor::ActorContext;
use crate::common::actor::RecordingSink;
use crate::common::services::Services;
use crate::common::state::State;
use crate::common::app_state::AppState;
use crate::feat::compaction_actor::CompactionActor;
use crate::feat::compaction_actor::CompactionActorDeps;
use crate::feat::compaction_actor::protocol::command::{CancelCompaction, CompactContext};
use crate::feat::session::protocol::history_appended::HistoryAppended;
use crate::protocol::{Command, SessionId};

#[test]
fn compaction_entry_is_compaction_returns_true() {
    let entry = ChatEntry {
        id: crate::protocol::ChatEntryId::new(),
        timestamp: jiff::Timestamp::now(),
        kind: ChatEntryKind::Compaction {
            summary: "test".to_owned(),
            tokens_before: 100,
            entries_compacted: 5,
            model_used: "test/model".to_owned(),
        },
        pin_position: None,
        context_override: crate::protocol::ContextOverride::Default,
    };
    assert!(entry.is_compaction());
}

#[test]
fn user_entry_is_compaction_returns_false() {
    let entry = ChatEntry::user("hello");
    assert!(!entry.is_compaction());
}

#[test]
fn insert_entry_at_places_entry_at_correct_position() {
    // Given a session with 3 entries.
    let mut session = crate::feat::session::chat_session::ChatSessionState::new();
    session.push_entry(ChatEntry::user("first"));
    session.push_entry(ChatEntry::assistant("second"));
    session.push_entry(ChatEntry::user("third"));

    assert_eq!(session.history().len(), 3);

    // When inserting at position 1.
    let idx = session.insert_entry_at(1, ChatEntry::system("inserted"));

    // Then the entry is at position 1 and others shifted.
    assert_eq!(idx, 1);
    assert_eq!(session.history().len(), 4);
    assert_eq!(session.history()[0].text(), "first");
    assert_eq!(session.history()[1].text(), "inserted");
    assert_eq!(session.history()[2].text(), "second");
    assert_eq!(session.history()[3].text(), "third");
}

#[test]
fn insert_entry_at_end_appends() {
    // Given a session with 2 entries.
    let mut session = crate::feat::session::chat_session::ChatSessionState::new();
    session.push_entry(ChatEntry::user("first"));
    session.push_entry(ChatEntry::assistant("second"));

    // When inserting at position 2 (end).
    let idx = session.insert_entry_at(2, ChatEntry::system("appended"));

    // Then the entry is appended.
    assert_eq!(idx, 2);
    assert_eq!(session.history().len(), 3);
    assert_eq!(session.history()[2].text(), "appended");
}

#[test]
fn mark_entries_ignored_sets_flag() {
    // Given a session with 4 entries.
    let mut session = crate::feat::session::chat_session::ChatSessionState::new();
    session.push_entry(ChatEntry::user("a"));
    session.push_entry(ChatEntry::assistant("b"));
    session.push_entry(ChatEntry::user("c"));
    session.push_entry(ChatEntry::assistant("d"));

    // When marking entries 0 and 1 as ignored.
    session.mark_entries_ignored(&[0, 1]);

    // Then those entries are ignored but others are not.
    assert!(session.history()[0].ignored());
    assert!(session.history()[1].ignored());
    assert!(!session.history()[2].ignored());
    assert!(!session.history()[3].ignored());
}

#[test]
fn mark_entries_ignored_with_pinned_entry() {
    // Given a session with a pinned entry.
    let mut session = crate::feat::session::chat_session::ChatSessionState::new();
    session.push_entry(ChatEntry::user("pinned").with_pin(PinPosition::Relative));
    session.push_entry(ChatEntry::assistant("response"));

    // When marking index 0 as ignored.
    session.mark_entries_ignored(&[0]);

    // Then the entry is marked ignored but still pinned.
    assert!(session.history()[0].ignored());
    assert!(session.history()[0].is_pinned());
    // Pin override works: pinned && ignored still counts as "included".
    assert!(session.history()[0].is_pinned() || !session.history()[0].ignored());
}

#[test]
fn serializer_skips_system_entries() {
    let entries = vec![
        ChatEntry::user("hello"),
        ChatEntry::system("status"),
        ChatEntry::assistant("hi"),
    ];
    let result = serialize_entries_for_compaction(&entries);
    assert!(!result.contains("status"));
    assert!(result.contains("[User]: hello"));
    assert!(result.contains("[Assistant]: hi"));
}

#[test]
fn vec_order_after_compaction_insertion() {
    // Given a session with entries that will be compacted.
    let mut session = crate::feat::session::chat_session::ChatSessionState::new();
    session.push_entry(ChatEntry::system("system")); // idx 0 — exempt
    session.push_entry(ChatEntry::user("old1")); // idx 1 — compacted
    session.push_entry(ChatEntry::assistant("old2")); // idx 2 — compacted
    session.push_entry(ChatEntry::user("recent1")); // idx 3 — kept (recent)
    session.push_entry(ChatEntry::assistant("recent2")); // idx 4 — kept (recent)

    // When marking entries 1,2 as ignored and inserting compaction at boundary.
    session.mark_entries_ignored(&[1, 2]);
    let compaction = ChatEntry {
        id: crate::protocol::ChatEntryId::new(),
        timestamp: jiff::Timestamp::now(),
        kind: ChatEntryKind::Compaction {
            summary: "summarized".to_owned(),
            tokens_before: 50,
            entries_compacted: 2,
            model_used: "test".to_owned(),
        },
        pin_position: None,
        context_override: crate::protocol::ContextOverride::Default,
    };
    session.insert_entry_at(3, compaction);

    // Then the vec is in correct logical order.
    assert_eq!(session.history().len(), 6);
    assert_eq!(session.history()[0].text(), "system"); // system (exempt)
    assert!(session.history()[1].ignored()); // old1 (compacted)
    assert!(session.history()[2].ignored()); // old2 (compacted)
    assert!(session.history()[3].is_compaction()); // compaction entry
    assert!(!session.history()[4].ignored()); // recent1 (kept)
    assert!(!session.history()[5].ignored()); // recent2 (kept)
}

#[test]
fn boundary_detection_finds_last_compaction() {
    // Given a session that already has a compaction entry.
    let mut session = crate::feat::session::chat_session::ChatSessionState::new();
    session.push_entry(ChatEntry::system("system"));
    session.push_entry(ChatEntry::user("old1"));
    // First compaction.
    session.push_entry(ChatEntry {
        id: crate::protocol::ChatEntryId::new(),
        timestamp: jiff::Timestamp::now(),
        kind: ChatEntryKind::Compaction {
            summary: "first compaction".to_owned(),
            tokens_before: 100,
            entries_compacted: 1,
            model_used: "test".to_owned(),
        },
        pin_position: None,
        context_override: crate::protocol::ContextOverride::Default,
    });
    // Entries after first compaction.
    session.push_entry(ChatEntry::user("new1"));
    session.push_entry(ChatEntry::assistant("new2"));

    // When looking for the start boundary (last compaction entry).
    let history = session.history();
    let start_index = history
        .iter()
        .rposition(super::super::session::chat_entry::ChatEntry::is_compaction)
        .map_or(0, |i| i + 1);

    // Then the boundary starts after the first compaction.
    assert_eq!(start_index, 3); // indices 3,4 are the new entries
    assert_eq!(history[start_index].text(), "new1");
}

#[test]
fn serializer_includes_tool_calls_and_results() {
    let entries = vec![
        ChatEntry::user("run it"),
        ChatEntry {
            id: crate::protocol::ChatEntryId::new(),
            timestamp: jiff::Timestamp::now(),
            kind: ChatEntryKind::ToolCall {
                id: "call1".to_owned(),
                name: "bash".to_owned(),
                arguments: "echo hello".to_owned(),
            },
            pin_position: None,
            context_override: crate::protocol::ContextOverride::Default,
        },
        ChatEntry {
            id: crate::protocol::ChatEntryId::new(),
            timestamp: jiff::Timestamp::now(),
            kind: ChatEntryKind::ToolResult {
                id: "call1".to_owned(),
                name: "bash".to_owned(),
                content: "hello".to_owned(),
                status: ToolResultStatus::Success,
                full_content: None,
                truncation: None,
            },
            pin_position: None,
            context_override: crate::protocol::ContextOverride::Default,
        },
    ];
    let result = serialize_entries_for_compaction(&entries);
    assert!(result.contains("[User]: run it"));
    assert!(result.contains("[Tool call]: bash"));
    assert!(result.contains("[Tool result] bash: hello"));
}

#[allow(clippy::cast_precision_loss)]
#[test]
fn auto_compaction_threshold_estimation() {
    use crate::feat::context::strategy::token_estimator::{
        CharRatioEstimator, estimate_entry_tokens,
    };
    use crate::feat::preferences_actor::user_preferences::CompactionConfig;

    // Given a session with entries and a threshold of 0.7 with budget 1000.
    let _config = CompactionConfig::default();
    let token_budget: usize = 1000;
    let threshold = 0.7;

    let session = {
        let mut session = crate::feat::session::chat_session::ChatSessionState::new();
        // Add entries that together exceed 700 estimated tokens.
        for i in 0..50 {
            session.push_entry(ChatEntry::user(format!(
                "message {i} with enough text to accumulate tokens"
            )));
            session.push_entry(ChatEntry::assistant(format!(
                "response {i} with enough text to accumulate tokens"
            )));
        }
        session
    };

    let estimator = CharRatioEstimator;
    let total_tokens: usize = session
        .history()
        .iter()
        .map(|e| estimate_entry_tokens(&estimator, e))
        .sum();

    let threshold_tokens = (threshold * token_budget as f64) as usize;

    // Then the total exceeds the threshold.
    assert!(
        total_tokens > threshold_tokens,
        "total tokens ({total_tokens}) should exceed threshold ({threshold_tokens})"
    );
}

#[allow(clippy::cast_precision_loss)]
#[test]
fn auto_compaction_no_trigger_below_threshold() {
    use crate::feat::context::strategy::token_estimator::{
        CharRatioEstimator, estimate_entry_tokens,
    };

    // Given a session with few entries (well below threshold).
    let token_budget: usize = 100_000;
    let threshold = 0.7;

    let session = {
        let mut session = crate::feat::session::chat_session::ChatSessionState::new();
        session.push_entry(ChatEntry::user("hi"));
        session.push_entry(ChatEntry::assistant("hello"));
        session
    };

    let estimator = CharRatioEstimator;
    let total_tokens: usize = session
        .history()
        .iter()
        .map(|e| estimate_entry_tokens(&estimator, e))
        .sum();

    let threshold_tokens = (threshold * token_budget as f64) as usize;

    // Then the total is below the threshold.
    assert!(
        total_tokens <= threshold_tokens,
        "total tokens ({total_tokens}) should be below threshold ({threshold_tokens})"
    );
}

// --- Auto-compaction deduplication tests ---

/// Helper: create a test actor with recording sink and a session configured
/// with a very low token budget so compaction threshold is easy to exceed.
///
/// Returns `(recording_sink, actor_context, actor, session_id)`.
fn test_actor_with_low_budget() -> (
    std::sync::Arc<RecordingSink>,
    ActorContext,
    super::CompactionActor,
    SessionId,
) {
    let sink = std::sync::Arc::new(RecordingSink::new());
    let ctx = ActorContext::new("test-compaction", sink.clone());

    // Build state with default config. Auto-compaction threshold = 0.7 * 150,000 = 105,000 tokens.
    // The provider registry is empty, so fallback_context_window (150,000) is used.
    let app_state = AppState::default();
    let state = State::new(app_state);
    let session_id = state.read().session.active_session_id().clone();

    let services = Services::new();
    let handle = services.handle.clone();
    let deps = CompactionActorDeps {
        state,
        services,
        handle,
    };
    let mut ctx = ctx;
    let actor = CompactionActor::activate(deps, &mut ctx);

    (sink, ctx, actor, session_id)
}

/// Helper: count `EnqueueCompaction` commands in the recording sink.
fn count_enqueue_compaction(commands: &[Command]) -> usize {
    commands
        .iter()
        .filter(|c| matches!(c, Command::EnqueueCompaction(_)))
        .count()
}

/// Helper: build a `HistoryAppended` payload with high token count.
fn high_token_event(session_id: &SessionId) -> HistoryAppended {
    HistoryAppended {
        session_id: session_id.clone(),
        total_estimated_tokens: 200_000, // well above threshold (0.7 * 150_000 = 105_000)
    }
}

#[rstest::rstest]
#[test]
fn double_history_appended_emits_single_enqueue_compaction() {
    // Given an actor with a low token budget.
    let (sink, ctx, mut actor, session_id) = test_actor_with_low_budget();

    let event = high_token_event(&session_id);

    // When sending HistoryAppended twice (simulating tool-result + stream-completion).
    actor.handle_history_appended(&event, &ctx);
    actor.handle_history_appended(&event, &ctx);

    // Then exactly one EnqueueCompaction was emitted.
    let commands = sink.commands();
    let count = count_enqueue_compaction(&commands);
    assert_eq!(
        count, 1,
        "expected exactly 1 EnqueueCompaction, got {count}"
    );
}

#[rstest::rstest]
#[test]
fn flag_resets_after_compact_context_allows_retrigger() {
    // Given an actor with a low token budget.
    let (sink, ctx, mut actor, session_id) = test_actor_with_low_budget();

    let event = high_token_event(&session_id);

    // When triggering auto-compaction.
    actor.handle_history_appended(&event, &ctx);
    assert_eq!(
        count_enqueue_compaction(&sink.commands()),
        1,
        "first trigger should emit one EnqueueCompaction"
    );

    // And then receiving CompactContext (simulates queue dispatching it).
    let compact_cmd = CompactContext {
        session_id: session_id.clone(),
        compact_all: false,
    };
    actor.handle_compact_context(&compact_cmd, &ctx);

    // And sending another HistoryAppended with tokens still above threshold.
    sink.clear();
    actor.handle_history_appended(&event, &ctx);

    // Then a new EnqueueCompaction is emitted (flag was reset).
    let count = count_enqueue_compaction(&sink.commands());
    assert_eq!(
        count, 1,
        "expected 1 EnqueueCompaction after reset, got {count}"
    );
}

#[rstest::rstest]
#[test]
fn flag_resets_after_cancel_compaction_allows_retrigger() {
    // Given an actor with a low token budget.
    let (sink, ctx, mut actor, session_id) = test_actor_with_low_budget();

    let event = high_token_event(&session_id);

    // When triggering auto-compaction.
    actor.handle_history_appended(&event, &ctx);
    assert_eq!(
        count_enqueue_compaction(&sink.commands()),
        1,
        "first trigger should emit one EnqueueCompaction"
    );

    // And then cancelling compaction.
    let cancel_cmd = CancelCompaction {
        session_id: session_id.clone(),
    };
    actor.handle_cancel_compaction(&cancel_cmd);

    // And sending another HistoryAppended with tokens still above threshold.
    sink.clear();
    actor.handle_history_appended(&event, &ctx);

    // Then a new EnqueueCompaction is emitted (flag was reset).
    let count = count_enqueue_compaction(&sink.commands());
    assert_eq!(
        count, 1,
        "expected 1 EnqueueCompaction after cancel reset, got {count}"
    );
}

// --- Tool-chain boundary detection tests ---

#[test]
fn cut_on_user_entry_needs_no_adjustment() {
    // Given history with User entries at clear boundaries.
    let history = vec![
        ChatEntry::user("msg1"),
        ChatEntry::assistant("resp1"),
        ChatEntry::user("msg2"),
        ChatEntry::assistant("resp2"),
    ];

    // When cut lands on a User entry.
    let result = super::adjust_cut_to_boundary(&history, 2);

    // Then no adjustment needed.
    assert_eq!(result, 2);
}

#[test]
fn cut_on_tool_call_walks_to_next_user() {
    // Given history with a tool call chain.
    let history = vec![
        ChatEntry::user("msg1"),
        ChatEntry::assistant("resp1"),
        ChatEntry::tool_call("tc1", "bash", "ls"),
        ChatEntry::tool_result("tc1", "bash", "file.txt", ToolResultStatus::Success),
        ChatEntry::assistant("done"),
        ChatEntry::user("msg2"),
    ];

    // When cut lands on ToolCall entry.
    let result = super::adjust_cut_to_boundary(&history, 2);

    // Then it walks forward to the next User entry.
    assert_eq!(result, 5);
}

#[test]
fn cut_on_tool_result_walks_to_next_user() {
    // Given history with a tool call chain.
    let history = vec![
        ChatEntry::user("msg1"),
        ChatEntry::assistant("resp1"),
        ChatEntry::tool_call("tc1", "bash", "ls"),
        ChatEntry::tool_result("tc1", "bash", "file.txt", ToolResultStatus::Success),
        ChatEntry::assistant("done"),
        ChatEntry::user("msg2"),
    ];

    // When cut lands on ToolResult entry.
    let result = super::adjust_cut_to_boundary(&history, 3);

    // Then it walks forward to the next User entry.
    assert_eq!(result, 5);
}

#[test]
fn cut_on_assistant_after_tool_result_walks_to_next_user() {
    // Given history with a tool call chain.
    let history = vec![
        ChatEntry::user("msg1"),
        ChatEntry::assistant("resp1"),
        ChatEntry::tool_call("tc1", "bash", "ls"),
        ChatEntry::tool_result("tc1", "bash", "file.txt", ToolResultStatus::Success),
        ChatEntry::assistant("done"),
        ChatEntry::user("msg2"),
    ];

    // When cut lands on the responding Assistant entry.
    let result = super::adjust_cut_to_boundary(&history, 4);

    // Then it walks forward to the next User entry.
    assert_eq!(result, 5);
}

#[test]
fn multiple_tool_call_batches_walks_past_all() {
    // Given history with multiple consecutive tool call batches.
    let history = vec![
        ChatEntry::user("msg1"),
        ChatEntry::assistant("resp1"),
        ChatEntry::tool_call("tc1", "bash", "ls"),
        ChatEntry::tool_result("tc1", "bash", "file.txt", ToolResultStatus::Success),
        ChatEntry::assistant("checking"),
        ChatEntry::tool_call("tc2", "read", "file.rs"),
        ChatEntry::tool_result("tc2", "read", "contents", ToolResultStatus::Success),
        ChatEntry::assistant("final"),
        ChatEntry::user("msg2"),
    ];

    // When cut lands on the first ToolCall.
    let result = super::adjust_cut_to_boundary(&history, 2);

    // Then it walks past the entire multi-batch chain to the next User.
    assert_eq!(result, 8);
}

#[test]
fn no_user_after_cut_returns_history_len() {
    // Given history ending with a tool chain (no trailing User).
    let history = vec![
        ChatEntry::user("msg1"),
        ChatEntry::assistant("resp1"),
        ChatEntry::tool_call("tc1", "bash", "ls"),
        ChatEntry::tool_result("tc1", "bash", "file.txt", ToolResultStatus::Success),
    ];

    // When cut lands on ToolCall with no User after it.
    let result = super::adjust_cut_to_boundary(&history, 2);

    // Then the entire history gets compacted.
    assert_eq!(result, 4);
}

#[test]
fn cut_at_end_returns_end() {
    // Given a simple history.
    let history = vec![
        ChatEntry::user("msg1"),
        ChatEntry::assistant("resp1"),
    ];

    // When cut is already at the end.
    let result = super::adjust_cut_to_boundary(&history, 2);

    // Then it stays at the end.
    assert_eq!(result, 2);
}

#[test]
fn cut_on_standalone_assistant_walks_to_next_user() {
    // Given history where the cut lands on a standalone Assistant.
    let history = vec![
        ChatEntry::user("msg1"),
        ChatEntry::assistant("resp1"),
        ChatEntry::assistant("resp2"),
        ChatEntry::user("msg2"),
    ];

    // When cut lands on a standalone Assistant (not in a tool chain).
    let result = super::adjust_cut_to_boundary(&history, 1);

    // Then it walks forward to the next User entry.
    assert_eq!(result, 3);
}

