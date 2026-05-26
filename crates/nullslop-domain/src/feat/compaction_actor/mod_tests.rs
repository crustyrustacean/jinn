#![allow(clippy::expect_used, clippy::indexing_slicing)]

//! Tests for the compaction actor.

use crate::feat::compaction_actor::serializer::serialize_entries_for_compaction;
use crate::feat::session::chat_entry::{ChatEntry, ChatEntryKind};
use crate::feat::session::tool_result_status::ToolResultStatus;
use crate::protocol::PinPosition;

use crate::common::actor::Actor;
use crate::common::actor::ActorContext;
use crate::common::actor::RecordingSink;
use crate::common::app_state::AppState;
use crate::common::services::Services;
use crate::common::state::State;
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
            tokens_after: 50,
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
            tokens_after: 25,
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
            tokens_after: 50,
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

    // Set a test compaction prompt so generate_summary doesn't get an empty prompt.
    state.write().context.compaction_prompt = "test compaction prompt".to_owned();

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
fn cut_on_tool_call_walks_to_next_assistant() {
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

    // Then it walks forward to the next non-tool entry (Assistant).
    assert_eq!(result, 4);
}

#[test]
fn cut_on_tool_result_walks_to_next_assistant() {
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

    // Then it walks forward to the next non-tool entry (Assistant).
    assert_eq!(result, 4);
}

#[test]
fn cut_on_assistant_after_tool_result_stays() {
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

    // Then it stays — Assistant is a valid cut point.
    assert_eq!(result, 4);
}

#[test]
fn multiple_tool_call_batches_walks_to_next_assistant() {
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

    // Then it walks to the next non-tool entry (Assistant at cycle boundary).
    assert_eq!(result, 4);
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
    let history = vec![ChatEntry::user("msg1"), ChatEntry::assistant("resp1")];

    // When cut is already at the end.
    let result = super::adjust_cut_to_boundary(&history, 2);

    // Then it stays at the end.
    assert_eq!(result, 2);
}

#[test]
fn cut_on_standalone_assistant_stays() {
    // Given history where the cut lands on a standalone Assistant.
    let history = vec![
        ChatEntry::user("msg1"),
        ChatEntry::assistant("resp1"),
        ChatEntry::assistant("resp2"),
        ChatEntry::user("msg2"),
    ];

    // When cut lands on a standalone Assistant (not in a tool chain).
    let result = super::adjust_cut_to_boundary(&history, 1);

    // Then it stays — Assistant is a valid cut point.
    assert_eq!(result, 1);
}

#[test]
fn pure_tool_loop_cut_on_tool_call_finds_assistant() {
    // Given a tool loop with no User messages after the initial one.
    let history = vec![
        ChatEntry::user("fix this bug"),
        ChatEntry::assistant("checking"),
        ChatEntry::tool_call("tc1", "bash", "ls"),
        ChatEntry::tool_result("tc1", "bash", "file.txt", ToolResultStatus::Success),
        ChatEntry::assistant("done"),
    ];

    // When cut lands on ToolCall entry.
    let result = super::adjust_cut_to_boundary(&history, 2);

    // Then it walks to the next Assistant (not history.len()).
    assert_eq!(result, 4);
}

#[test]
fn pure_tool_loop_cut_on_tool_result_finds_assistant() {
    // Given a tool loop with no User messages after the initial one.
    let history = vec![
        ChatEntry::user("fix this bug"),
        ChatEntry::assistant("checking"),
        ChatEntry::tool_call("tc1", "bash", "ls"),
        ChatEntry::tool_result("tc1", "bash", "file.txt", ToolResultStatus::Success),
        ChatEntry::assistant("done"),
    ];

    // When cut lands on ToolResult entry.
    let result = super::adjust_cut_to_boundary(&history, 3);

    // Then it walks to the next Assistant.
    assert_eq!(result, 4);
}

#[test]
fn cut_on_assistant_in_tool_loop_stays() {
    // Given a tool loop ending with Assistant.
    let history = vec![
        ChatEntry::user("fix this bug"),
        ChatEntry::assistant("checking"),
        ChatEntry::tool_call("tc1", "bash", "ls"),
        ChatEntry::tool_result("tc1", "bash", "file.txt", ToolResultStatus::Success),
        ChatEntry::assistant("done"),
    ];

    // When cut lands on Assistant.
    let result = super::adjust_cut_to_boundary(&history, 4);

    // Then it stays — Assistant is a valid cut point.
    assert_eq!(result, 4);
}

#[test]
fn tool_loop_no_trailing_assistant_returns_len() {
    // Given a tool loop ending with ToolResult (no trailing Assistant).
    let history = vec![
        ChatEntry::user("fix this bug"),
        ChatEntry::assistant("checking"),
        ChatEntry::tool_call("tc1", "bash", "ls"),
        ChatEntry::tool_result("tc1", "bash", "file.txt", ToolResultStatus::Success),
    ];

    // When cut lands on ToolCall with no non-tool entry after it.
    let result = super::adjust_cut_to_boundary(&history, 2);

    // Then it returns history.len() — compact everything.
    assert_eq!(result, 4);
}

#[test]
fn cut_between_tool_cycles_finds_assistant() {
    // Given multiple tool cycles with no User messages.
    let history = vec![
        ChatEntry::user("fix this bug"),
        ChatEntry::assistant("checking"),
        ChatEntry::tool_call("tc1", "bash", "ls"),
        ChatEntry::tool_result("tc1", "bash", "file.txt", ToolResultStatus::Success),
        ChatEntry::assistant("reading file"),
        ChatEntry::tool_call("tc2", "read", "file.rs"),
        ChatEntry::tool_result("tc2", "read", "contents", ToolResultStatus::Success),
        ChatEntry::assistant("final fix"),
    ];

    // When cut lands on ToolResult between cycles.
    let result = super::adjust_cut_to_boundary(&history, 3);

    // Then it walks to the next Assistant (start of next cycle).
    assert_eq!(result, 4);
}

#[test]
fn cut_on_error_is_safe() {
    // Given history with an Error entry after tool chain.
    let history = vec![
        ChatEntry::user("fix this bug"),
        ChatEntry::assistant("checking"),
        ChatEntry::tool_call("tc1", "bash", "ls"),
        ChatEntry::tool_result("tc1", "bash", "file.txt", ToolResultStatus::Success),
        ChatEntry::error("something went wrong"),
    ];

    // When cut lands on Error.
    let result = super::adjust_cut_to_boundary(&history, 4);

    // Then it stays — Error is a valid cut point.
    assert_eq!(result, 4);
}

#[test]
fn cut_on_compaction_entry_is_safe() {
    // Given history with a Compaction entry after tool chain.
    let history = vec![
        ChatEntry::user("fix this bug"),
        ChatEntry::assistant("checking"),
        ChatEntry::tool_call("tc1", "bash", "ls"),
        ChatEntry::tool_result("tc1", "bash", "file.txt", ToolResultStatus::Success),
        ChatEntry {
            id: crate::protocol::ChatEntryId::new(),
            timestamp: jiff::Timestamp::now(),
            kind: ChatEntryKind::Compaction {
                summary: "previous summary".to_owned(),
                tokens_before: 100,
                tokens_after: 50,
                entries_compacted: 2,
                model_used: "test".to_owned(),
            },
            pin_position: None,
            context_override: crate::protocol::ContextOverride::Default,
        },
    ];

    // When cut lands on Compaction.
    let result = super::adjust_cut_to_boundary(&history, 4);

    // Then it stays — Compaction is a valid cut point.
    assert_eq!(result, 4);
}

#[test]
fn long_tool_loop_finds_nearest_assistant() {
    // Given a long autonomous tool loop with many cycles.
    let history = vec![
        ChatEntry::user("fix this bug"),
        ChatEntry::assistant("cycle a"),
        ChatEntry::tool_call("tc1", "bash", "ls"),
        ChatEntry::tool_result("tc1", "bash", "file.txt", ToolResultStatus::Success),
        ChatEntry::assistant("cycle b"),
        ChatEntry::tool_call("tc2", "read", "file.rs"),
        ChatEntry::tool_result("tc2", "read", "contents", ToolResultStatus::Success),
        ChatEntry::assistant("cycle c"),
        ChatEntry::tool_call("tc3", "bash", "cat other.txt"),
        ChatEntry::tool_result("tc3", "bash", "other contents", ToolResultStatus::Success),
        ChatEntry::assistant("final fix"),
    ];

    // When cut lands on ToolCall tc2 (index 5).
    let result = super::adjust_cut_to_boundary(&history, 5);

    // Then it walks to the nearest Assistant — "cycle c" at index 7.
    // The reserve window keeps: Assistant("cycle c") → ToolCall(tc3) → ToolResult(tc3) → Assistant("final fix").
    assert_eq!(result, 7);
}

// --- Pass 2: Incomplete tool loop defense-in-depth ---

#[test]
fn adjust_cut_walks_past_incomplete_tool_loop() {
    // Given history ending with Assistant + dangling ToolCall (no ToolResult).
    let history = vec![
        ChatEntry::user("run it"),
        ChatEntry::assistant(""),
        ChatEntry::tool_call("tc1", "bash", "ls"),
    ];

    // When cut lands on the empty Assistant at index 1.
    let result = super::adjust_cut_to_boundary(&history, 1);

    // Then it walks past the incomplete tool loop — returns history.len().
    assert_eq!(result, 3);
}

#[test]
fn adjust_cut_stays_at_complete_tool_loop() {
    // Given history with a complete tool loop.
    let history = vec![
        ChatEntry::user("run it"),
        ChatEntry::assistant(""),
        ChatEntry::tool_call("tc1", "bash", "ls"),
        ChatEntry::tool_result("tc1", "bash", "file.txt", ToolResultStatus::Success),
    ];

    // When cut lands on the Assistant at index 1.
    let result = super::adjust_cut_to_boundary(&history, 1);

    // Then it stays — the tool loop is complete.
    assert_eq!(result, 1);
}

#[test]
fn adjust_cut_walks_past_incomplete_then_lands_on_user() {
    // Given history with incomplete loop then a User entry.
    let history = vec![
        ChatEntry::user("run it"),
        ChatEntry::assistant(""),
        ChatEntry::tool_call("tc1", "bash", "ls"),
        ChatEntry::user("next"),
    ];

    // When cut lands on the Assistant at index 1.
    let result = super::adjust_cut_to_boundary(&history, 1);

    // Then it walks past the incomplete loop and lands on the User entry.
    assert_eq!(result, 3);
}

#[test]
fn adjust_cut_handles_multiple_incomplete_loops() {
    // Given history with two consecutive incomplete tool loops then a User entry.
    let history = vec![
        ChatEntry::user("run it"),
        ChatEntry::assistant(""),
        ChatEntry::tool_call("tc1", "bash", "ls"),
        ChatEntry::assistant(""),
        ChatEntry::tool_call("tc2", "read", "a.rs"),
        ChatEntry::user("next"),
    ];

    // When cut lands on the first Assistant at index 1.
    let result = super::adjust_cut_to_boundary(&history, 1);

    // Then it walks past both incomplete loops and lands on the User entry.
    assert_eq!(result, 5);
}

#[test]
fn adjust_cut_all_complete_tool_loops_no_change() {
    // Given history with multiple complete tool loops.
    let history = vec![
        ChatEntry::user("run it"),
        ChatEntry::assistant(""),
        ChatEntry::tool_call("tc1", "bash", "ls"),
        ChatEntry::tool_result("tc1", "bash", "file.txt", ToolResultStatus::Success),
        ChatEntry::assistant("checking"),
        ChatEntry::tool_call("tc2", "read", "a.rs"),
        ChatEntry::tool_result("tc2", "read", "contents", ToolResultStatus::Success),
    ];

    // When cut lands on first Assistant at index 1.
    let result = super::adjust_cut_to_boundary(&history, 1);

    // Then it stays - the tool loop is complete.
    assert_eq!(result, 1);
}

#[test]
fn adjust_cut_non_empty_assistant_with_incomplete_loop_walks_forward() {
    // Given history with a non-empty Assistant followed by a dangling ToolCall.
    let history = vec![
        ChatEntry::user("run it"),
        ChatEntry::assistant("let me check"),
        ChatEntry::tool_call("tc1", "bash", "ls"),
        ChatEntry::user("next"),
    ];

    // When cut lands on the non-empty Assistant at index 1.
    let result = super::adjust_cut_to_boundary(&history, 1);

    // Then it walks past the incomplete loop to the User entry.
    assert_eq!(result, 3);
}

// --- Cut-point algorithm tests for reserve and compact_all ---

#[test]
fn cut_index_defaults_to_start_when_tokens_within_reserve() {
    use crate::feat::context::strategy::token_estimator::{
        CharRatioEstimator, estimate_entry_tokens,
    };

    // Given history with total tokens well below the 20,000 reserve default.
    let history = vec![
        ChatEntry::user("hello"),
        ChatEntry::assistant("hi there"),
        ChatEntry::user("how are you?"),
        ChatEntry::assistant("doing well"),
    ];

    // When calculating the cut point with compact_all=false (the default),
    // walking backwards accumulating tokens never exceeds the reserve.
    let start_index = 0;
    let reserve_tokens = 20_000;
    let compact_all = false;

    let estimator = CharRatioEstimator;
    let mut accumulated_tokens = 0usize;
    let mut cut_index = if compact_all {
        history.len()
    } else {
        start_index
    };

    if !compact_all {
        for i in (start_index..history.len()).rev() {
            let entry = &history[i];
            let tokens = estimate_entry_tokens(&estimator, entry);
            accumulated_tokens += tokens;
            if accumulated_tokens > reserve_tokens {
                cut_index = i + 1;
                break;
            }
        }
    }

    let cut_index = super::adjust_cut_to_boundary(&history, cut_index);

    // Then cut_index stays at start_index (0) — nothing to compact.
    assert_eq!(
        cut_index, start_index,
        "cut_index should be start_index when all tokens fit within reserve"
    );
}

#[test]
fn cut_index_equals_history_len_when_compact_all() {
    // Given history with tokens well below the reserve.
    let history = vec![
        ChatEntry::user("hello"),
        ChatEntry::assistant("hi there"),
        ChatEntry::user("how are you?"),
        ChatEntry::assistant("doing well"),
    ];

    // When compact_all=true, cut_index starts at history.len() regardless of reserve.
    let start_index = 0;
    let compact_all = true;

    let cut_index = if compact_all {
        history.len()
    } else {
        start_index
    };

    let cut_index = super::adjust_cut_to_boundary(&history, cut_index);

    // Then cut_index equals history.len() — everything gets compacted.
    assert_eq!(
        cut_index,
        history.len(),
        "cut_index should be history.len() when compact_all=true"
    );
}

// --- compute_cut_index pure function tests ---

#[test]
fn compute_cut_index_returns_start_when_tokens_within_reserve() {
    // Given a history with total tokens below the reserve.
    let history = vec![
        ChatEntry::user("hi"),
        ChatEntry::assistant("hello"),
    ];

    // When computing cut index with a very large reserve.
    let cut_index = super::compute_cut_index(&history, 0, 100_000, false);

    // Then cut_index stays at start_index (0) — nothing to compact.
    assert_eq!(cut_index, 0);
}

#[test]
fn compute_cut_index_returns_len_when_compact_all() {
    // Given any history.
    let history = vec![
        ChatEntry::user("hi"),
        ChatEntry::assistant("hello"),
    ];

    // When computing cut index with compact_all=true.
    let cut_index = super::compute_cut_index(&history, 0, 100_000, true);

    // Then cut_index equals history length.
    assert_eq!(cut_index, history.len());
}

#[test]
fn compute_cut_index_walks_backwards_past_reserve() {
    use crate::feat::context::strategy::token_estimator::{CharRatioEstimator, estimate_entry_tokens};

    // Given a history with entries that have known token counts.
    // CharRatioEstimator: graphemes / 4 + 1
    // Each entry with 80 chars → 21 tokens. 10 entries = 210 tokens total.
    let long_msg = "a".repeat(80);
    let history: Vec<ChatEntry> = (0..10)
        .map(|i| {
            if i % 2 == 0 {
                ChatEntry::user(long_msg.clone())
            } else {
                ChatEntry::assistant(long_msg.clone())
            }
        })
        .collect();

    // Calculate total tokens to find a good reserve value.
    let estimator = CharRatioEstimator;
    let total_tokens: usize = history.iter().map(|e| estimate_entry_tokens(&estimator, e)).sum();

    // Reserve slightly less than half the tokens so the cut is unambiguous.
    let reserve = total_tokens / 2 - 1;

    // When computing cut index.
    let cut_index = super::compute_cut_index(&history, 0, reserve, false);

    // Then cut_index is somewhere in the middle (not 0, not len).
    assert!(cut_index > 0, "cut_index should be > 0, got {cut_index}");
    assert!(cut_index < history.len(), "cut_index should be < len, got {cut_index}");

    // And the entries before the cut represent more than the reserve.
    // Verify by checking that the "old" entries (before cut) are more than the reserve.
    let tokens_before_cut: usize = history[..cut_index]
        .iter()
        .map(|e| estimate_entry_tokens(&estimator, e))
        .sum();
    let tokens_after_cut: usize = history[cut_index..]
        .iter()
        .map(|e| estimate_entry_tokens(&estimator, e))
        .sum();
    // The "old" side (before cut) should be strictly larger than the reserve.
    assert!(
        tokens_before_cut >= reserve,
        "tokens before cut ({tokens_before_cut}) should be >= reserve ({reserve})"
    );
    // The "recent" side (after cut) should fit within the reserve.
    assert!(
        tokens_after_cut <= reserve + estimate_entry_tokens(&estimator, &history[0]),
        "tokens after cut ({tokens_after_cut}) should be close to reserve ({reserve})"
    );
}

#[test]
fn compute_cut_index_with_nonzero_start_index() {
    // Given a history where start_index > 0.
    let history = vec![
        ChatEntry::user("old1"),
        ChatEntry::assistant("old2"),
        ChatEntry::user("recent1"),
        ChatEntry::assistant("recent2"),
    ];

    // When computing cut index starting from index 2 with a large reserve.
    let cut_index = super::compute_cut_index(&history, 2, 100_000, false);

    // Then cut_index stays at start_index — all recent tokens fit in reserve.
    assert_eq!(cut_index, 2);
}

// --- gather_compactable_entries pure function tests ---

#[test]
fn gather_excludes_system_and_compaction_entries() {
    // Given history with System and Compaction entries mixed in.
    let history = vec![
        ChatEntry::user("msg1"),
        ChatEntry::system("system_msg"),
        ChatEntry::assistant("resp1"),
        ChatEntry {
            id: crate::protocol::ChatEntryId::new(),
            timestamp: jiff::Timestamp::now(),
            kind: ChatEntryKind::Compaction {
                summary: "summary".to_owned(),
                tokens_before: 100,
                tokens_after: 50,
                entries_compacted: 2,
                model_used: "test".to_owned(),
            },
            pin_position: None,
            context_override: crate::protocol::ContextOverride::Default,
        },
        ChatEntry::user("msg2"),
    ];

    // When gathering from 0 to 5.
    let (indices, tokens) = super::gather_compactable_entries(&history, 0, 5);

    // Then only non-system, non-compaction entries are gathered.
    assert_eq!(indices, vec![0, 2, 4]);
    assert!(tokens > 0, "tokens_before should be positive, got {tokens}");
}

#[test]
fn gather_returns_empty_when_all_exempt() {
    // Given history with only System entries.
    let history = vec![
        ChatEntry::system("s1"),
        ChatEntry::system("s2"),
    ];

    // When gathering.
    let (indices, tokens) = super::gather_compactable_entries(&history, 0, 2);

    // Then nothing is gathered.
    assert!(indices.is_empty());
    assert_eq!(tokens, 0);
}

#[test]
fn gather_tokens_before_matches_expected_sum() {
    use crate::feat::context::strategy::token_estimator::{CharRatioEstimator, estimate_entry_tokens};

    // Given history with 3 user entries.
    let history = vec![
        ChatEntry::user("hello"),
        ChatEntry::assistant("world"),
        ChatEntry::user("test"),
    ];

    // When gathering all.
    let (indices, tokens_before) = super::gather_compactable_entries(&history, 0, 3);

    // Then tokens_before matches the manually computed sum.
    let estimator = CharRatioEstimator;
    let expected: usize = history.iter().map(|e| estimate_entry_tokens(&estimator, e)).sum();
    assert_eq!(indices, vec![0, 1, 2]);
    assert_eq!(tokens_before, expected);
}

#[test]
fn gather_respects_start_and_cut_bounds() {
    // Given history with 5 entries.
    let history = vec![
        ChatEntry::user("a"),
        ChatEntry::assistant("b"),
        ChatEntry::user("c"),
        ChatEntry::assistant("d"),
        ChatEntry::user("e"),
    ];

    // When gathering from index 2 to 4.
    let (indices, _tokens) = super::gather_compactable_entries(&history, 2, 4);

    // Then only entries 2 and 3 are gathered (start..cut).
    assert_eq!(indices, vec![2, 3]);
}

// --- handle_history_appended threshold boundary tests ---

#[rstest::rstest]
#[test]
fn history_appended_does_not_trigger_at_exact_threshold() {
    // Given an actor with default config.
    // Default: threshold=0.7, fallback_context_window=150_000
    // threshold_tokens = 0.7 * 150_000 = 105_000
    let (sink, ctx, mut actor, session_id) = test_actor_with_low_budget();

    // When sending HistoryAppended with tokens exactly at the threshold.
    let event = HistoryAppended {
        session_id: session_id.clone(),
        total_estimated_tokens: 105_000, // exactly threshold * context_window
    };
    actor.handle_history_appended(&event, &ctx);

    // Then NO EnqueueCompaction is emitted (uses strict >).
    let count = count_enqueue_compaction(&sink.commands());
    assert_eq!(count, 0, "should not trigger at exact threshold");
}

#[rstest::rstest]
#[test]
fn history_appended_triggers_above_threshold() {
    // Given an actor with default config.
    let (sink, ctx, mut actor, session_id) = test_actor_with_low_budget();

    // When sending HistoryAppended with tokens one above the threshold.
    let event = HistoryAppended {
        session_id: session_id.clone(),
        total_estimated_tokens: 105_001, // one above threshold
    };
    actor.handle_history_appended(&event, &ctx);

    // Then one EnqueueCompaction is emitted.
    let count = count_enqueue_compaction(&sink.commands());
    assert_eq!(count, 1, "should trigger above threshold");
}

// --- perform_compaction end-to-end tests ---

/// Helper: create a full test environment for perform_compaction.
///
/// Sets up state with many entries (exceeding the 20K reserve) and a fake LLM
/// that returns a non-empty summary.
fn test_compaction_env() -> (
    std::sync::Arc<RecordingSink>,
    ActorContext,
    super::CompactionActor,
    SessionId,
) {
    let sink = std::sync::Arc::new(RecordingSink::new());
    let ctx = ActorContext::new("test-compaction-e2e", sink.clone());

    // Build state with a session full of entries.
    let app_state = AppState::default();
    let state = State::new(app_state);
    let session_id = state.read().session.active_session_id().clone();

    // Set a compaction prompt.
    state.write().context.compaction_prompt = "Summarize the conversation.".to_owned();

    // Add enough entries to exceed the 20K reserve.
    // CharRatioEstimator: each 80-char entry ≈ 21 tokens.
    // 20_000 / 21 ≈ 953 entries needed. Use 1500 to be safe.
    let long_msg = "x".repeat(80);
    {
        let mut state_guard = state.write();
        let session = state_guard.session.active_session_mut();
        for i in 0..1500 {
            if i % 2 == 0 {
                session.push_entry(ChatEntry::user(long_msg.clone()));
            } else {
                session.push_entry(ChatEntry::assistant(long_msg.clone()));
            }
        }
    }

    // Create services with a fake LLM that returns actual tokens.
    use crate::feat::provider_infra::FakeLlmServiceFactory;
    use crate::feat::provider_infra::LlmServiceFactoryService;
    let fake_factory = FakeLlmServiceFactory::new(vec![
        "Summary of the conversation so far.".to_owned(),
    ]);
    let mut services = Services::new();
    services.llm_service = LlmServiceFactoryService::new(std::sync::Arc::new(fake_factory));
    let handle = services.handle.clone();

    let deps = super::CompactionActorDeps {
        state,
        services,
        handle,
    };
    let mut ctx = ctx;
    let actor = super::CompactionActor::activate(deps, &mut ctx);

    (sink, ctx, actor, session_id)
}

/// Helper: find the first EndCompaction command in the recording sink.
fn find_end_compaction(commands: &[Command]) -> Option<super::protocol::command::EndCompaction> {
    for cmd in commands {
        if let Command::EndCompaction(payload) = cmd {
            return Some(payload.clone());
        }
    }
    None
}

#[rstest::rstest]
#[test]
fn perform_compaction_compacts_entries_and_returns_nonzero() {
    // Given a session with many entries and a fake LLM.
    let (sink, _ctx, mut actor, session_id) = test_compaction_env();

    // When triggering compaction.
    let compact_cmd = super::protocol::command::CompactContext {
        session_id: session_id.clone(),
        compact_all: false,
    };
    actor.handle_compact_context(&compact_cmd, &_ctx);

    // Then wait for the spawned task to complete (poll the sink).
    let mut end_cmd = None;
    for _ in 0..100 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        end_cmd = find_end_compaction(&sink.commands());
        if end_cmd.is_some() {
            break;
        }
    }
    let end_cmd = end_cmd.expect("EndCompaction should be emitted within 5 seconds");

    // And entries_compacted is > 0 (kills Ok(0) mutant).
    assert!(
        end_cmd.result.is_some(),
        "EndCompaction should have a result"
    );
    let result = end_cmd.result.unwrap();
    assert!(
        result.entries_compacted > 0,
        "should compact at least 1 entry, got {}",
        result.entries_compacted
    );
    // And the summary is non-empty and not "xyzzy" (kills Ok(String::new()) and Ok("xyzzy".into())).
    assert!(
        !result.summary.is_empty(),
        "summary should not be empty"
    );
    assert_ne!(
        result.summary, "xyzzy",
        "summary should not be xyzzy"
    );
    // And tokens_before > tokens_after (summary is shorter than original).
    assert!(
        result.tokens_before > result.tokens_after,
        "tokens_before ({}) should be > tokens_after ({})",
        result.tokens_before, result.tokens_after
    );
}

#[rstest::rstest]
#[test]
fn perform_compaction_compact_all_compacts_everything() {
    // Given a session with many entries and a fake LLM.
    let (sink, _ctx, mut actor, session_id) = test_compaction_env();

    // When triggering compaction with compact_all=true.
    let compact_cmd = super::protocol::command::CompactContext {
        session_id: session_id.clone(),
        compact_all: true,
    };
    actor.handle_compact_context(&compact_cmd, &_ctx);

    // Then wait for completion.
    let mut end_cmd = None;
    for _ in 0..100 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        end_cmd = find_end_compaction(&sink.commands());
        if end_cmd.is_some() {
            break;
        }
    }
    let end_cmd = end_cmd.expect("EndCompaction should be emitted");

    // And all entries are compacted (kills Ok(1) mutant — we have 1500 entries).
    let result = end_cmd.result.expect("should have a result");
    assert!(
        result.entries_compacted > 1,
        "compact_all should compact more than 1 entry, got {}",
        result.entries_compacted
    );
}
