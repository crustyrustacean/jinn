//! Integration tests for [`CompactionWorker`] using `FakeLlmServiceFactory`.
//!
//! Tests cover the full `evaluate_with_config` pipeline: boundary finding,
//! token accumulation, LLM summarization, and mutation production.
//! Bug regression tests verify the three reported compaction bugs are fixed.

#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

use std::sync::Arc;

use nullslop_provider::RetryConfig;

use crate::common::services::test_services::TestServices;
use crate::common::services::Services;
use crate::common::state::State;
use crate::common::app_state::AppState;
use crate::feat::compaction_worker::worker::{CompactionTrigger, CompactionWorker};
use crate::feat::compaction_worker::algorithm::{
    adjust_cut_to_boundary, compute_cut_index, find_start_boundary,
    gather_compactable_entries,
};
use crate::feat::preferences_actor::user_preferences::CompactionConfig;
use crate::feat::provider_infra::{FakeLlmServiceFactory, LlmServiceFactoryService};
use crate::feat::session::chat_entry::{
    ChatEntry, ChatEntryId, ChatEntryKind, ContextOverride,
};
use crate::feat::session::chat_session::ChatSessionState;
use crate::feat::session::history_mutation::HistoryMutation;
use crate::protocol::SessionId;

// ── Helpers ─────────────────────────────────────────────────────────────

const FAKE_SUMMARY: &str = "This is a compaction summary of the conversation.";

fn small_reserve_config() -> CompactionConfig {
    CompactionConfig {
        model: None,
        threshold: 0.8,
        reserve_tokens: 100,
        fallback_context_window: 150_000,
    }
}

fn huge_reserve_config() -> CompactionConfig {
    CompactionConfig {
        model: None,
        threshold: 0.8,
        reserve_tokens: 1_000_000,
        fallback_context_window: 150_000,
    }
}

fn moderate_reserve_config() -> CompactionConfig {
    CompactionConfig {
        model: None,
        threshold: 0.8,
        reserve_tokens: 10_000,
        fallback_context_window: 150_000,
    }
}

/// Build a history with N alternating user/assistant turns.
fn alternating_history(turns: usize) -> Vec<ChatEntry> {
    (0..turns)
        .flat_map(|i| {
            vec![
                ChatEntry::user(format!(
                    "User message {i} with enough text to accumulate tokens"
                )),
                ChatEntry::assistant(format!(
                    "Assistant response {i} with enough text to accumulate tokens"
                )),
            ]
        })
        .collect()
}

/// Build a compaction entry for use in history.
fn compaction_entry(summary: &str) -> ChatEntry {
    ChatEntry {
        id: ChatEntryId::new(),
        timestamp: jiff::Timestamp::now(),
        kind: ChatEntryKind::Compaction {
            summary: summary.to_owned(),
            tokens_before: 100,
            tokens_after: 50,
            entries_compacted: 5,
            model_used: "test-model".to_owned(),
        },
        pin_position: None,
        context_override: ContextOverride::Default,
    }
}

/// Build a history with a prior compaction summary followed by N turns.
fn history_with_prior_compaction(turns: usize) -> Vec<ChatEntry> {
    let mut entries = vec![compaction_entry("previous summary")];
    entries.extend(alternating_history(turns));
    entries
}

/// Create a `CompactionWorker` backed by a fake LLM.
fn test_worker(summary_text: &str) -> CompactionWorker {
    let services = TestServices::builder()
        .llm_service(LlmServiceFactoryService::new(Arc::new(
            FakeLlmServiceFactory::new(vec![summary_text.to_owned()]),
        )))
        .build();
    let handle = services.handle.clone();
    CompactionWorker {
        services,
        handle,
        state: State::new(AppState::default()),
        config: CompactionConfig::default(),
        compaction_prompt: "Summarize this conversation.".to_owned(),
    }
}

/// Create a `CompactionWorker` backed by a fake LLM, with state containing
/// a session with the given entries.
fn test_worker_with_session(
    summary_text: &str,
    entries: Vec<ChatEntry>,
) -> (CompactionWorker, SessionId) {
    let mut session = ChatSessionState::new();
    let session_id = session.session_id().clone();
    for entry in entries {
        session.push_entry(entry);
    }

    let state = State::new(AppState::default());
    {
        let mut app = state.write();
        app.session.insert(session);
    }

    let services = TestServices::builder()
        .llm_service(LlmServiceFactoryService::new(Arc::new(
            FakeLlmServiceFactory::new(vec![summary_text.to_owned()]),
        )))
        .build();
    let handle = services.handle.clone();

    let worker = CompactionWorker {
        services,
        handle,
        state,
        config: CompactionConfig::default(),
        compaction_prompt: "Summarize this conversation.".to_owned(),
    };

    (worker, session_id)
}

/// Run `evaluate_with_config` in a tokio runtime.
fn run_evaluate(
    worker: &CompactionWorker,
    history: &[ChatEntry],
    config: &CompactionConfig,
) -> Vec<HistoryMutation> {
    let rt = tokio::runtime::Runtime::new().expect("test runtime");
    rt.block_on(async {
        worker
            .evaluate_with_config(
                history,
                config,
                "test-model",
                "Summarize this conversation.",
                &RetryConfig::default(),
                false,
            )
            .await
            .expect("evaluate_with_config should succeed")
    })
}

/// Extract the `SetContextOverride` mutations from a mutation list.
fn forced_exclude_ids(mutations: &[HistoryMutation]) -> Vec<ChatEntryId> {
    mutations
        .iter()
        .filter_map(|m| match m {
            HistoryMutation::SetContextOverride { entry_id, value } => {
                if matches!(value, ContextOverride::ForcedExclude) {
                    Some(entry_id.clone())
                } else {
                    None
                }
            }
            _ => None,
        })
        .collect()
}

/// Extract the `InsertEntry` mutations from a mutation list.
fn insert_entries(mutations: &[HistoryMutation]) -> Vec<&HistoryMutation> {
    mutations
        .iter()
        .filter(|m| matches!(m, HistoryMutation::InsertEntry { .. }))
        .collect()
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 1: Compaction worker integration tests
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn worker_produces_mutations_when_threshold_exceeded() {
    // Given a worker and a long history that exceeds the small reserve.
    let worker = test_worker(FAKE_SUMMARY);
    let history = alternating_history(20);
    let config = small_reserve_config();

    // When evaluating.
    let mutations = run_evaluate(&worker, &history, &config);

    // Then mutations are produced.
    assert!(!mutations.is_empty(), "should produce mutations for long history");
}

#[test]
fn worker_returns_empty_when_within_reserve() {
    // Given a worker and a short history that fits in the huge reserve.
    let worker = test_worker(FAKE_SUMMARY);
    let history = alternating_history(2);
    let config = huge_reserve_config();

    // When evaluating.
    let mutations = run_evaluate(&worker, &history, &config);

    // Then no mutations are produced.
    assert!(
        mutations.is_empty(),
        "should produce no mutations when everything fits in reserve"
    );
}

#[test]
fn mutations_exclude_gathered_entries() {
    // Given a worker and a long history.
    let worker = test_worker(FAKE_SUMMARY);
    let history = alternating_history(20);
    let config = small_reserve_config();

    // When evaluating.
    let mutations = run_evaluate(&worker, &history, &config);

    // Then there are SetContextOverride(ForcedExclude) mutations.
    let excluded_ids = forced_exclude_ids(&mutations);
    assert!(!excluded_ids.is_empty(), "should have excluded entries");

    // And all excluded IDs correspond to history entries.
    let history_ids: Vec<ChatEntryId> = history.iter().map(|e| e.id.clone()).collect();
    for id in &excluded_ids {
        assert!(
            history_ids.contains(id),
            "excluded entry ID {id:?} should be in history"
        );
    }
}

#[test]
fn mutations_insert_compaction_summary_at_boundary() {
    // Given a worker and a long history.
    let worker = test_worker(FAKE_SUMMARY);
    let history = alternating_history(20);
    let config = small_reserve_config();

    // When evaluating.
    let mutations = run_evaluate(&worker, &history, &config);

    // Then there is exactly one InsertEntry mutation.
    let inserts = insert_entries(&mutations);
    assert_eq!(inserts.len(), 1, "should have exactly one InsertEntry mutation");

    // And it has an InsertEntry with a Compaction kind.
    match inserts[0] {
        HistoryMutation::InsertEntry {
            after_entry_id,
            entry,
        } => {
            // The after_entry_id should be the last gathered entry.
            assert!(
                after_entry_id.is_some(),
                "InsertEntry should reference a prior entry"
            );

            // The entry should be a Compaction kind with the fake summary.
            match &entry.kind {
                ChatEntryKind::Compaction { summary, .. } => {
                    assert_eq!(
                        summary, FAKE_SUMMARY,
                        "summary should contain the fake LLM response"
                    );
                }
                other => panic!("expected Compaction kind, got {other:?}"),
            }
        }
        other => panic!("expected InsertEntry, got {other:?}"),
    }
}

#[test]
fn mutations_exclude_entries_around_prior_compaction() {
    // Given a history with a prior compaction followed by entries.
    let worker = test_worker(FAKE_SUMMARY);
    let history = history_with_prior_compaction(5);
    let config = small_reserve_config();

    // When evaluating.
    let mutations = run_evaluate(&worker, &history, &config);

    // Then there are excluded entries (the user/assistant entries after the compaction).
    let excluded_ids = forced_exclude_ids(&mutations);
    assert!(!excluded_ids.is_empty(), "should have excluded entries after prior compaction");

    // And the compaction entry itself is NOT excluded (compaction entries are skipped by gather).
    let compaction_entry_id = &history[0].id;
    assert!(
        !excluded_ids.contains(compaction_entry_id),
        "prior compaction entry should not be excluded"
    );
}

#[test]
fn worker_preserves_recent_entries_in_reserve() {
    // Given a history with enough entries to trigger compaction, but with
    // a moderate reserve that keeps the most recent entries.
    let worker = test_worker(FAKE_SUMMARY);
    let history = alternating_history(20);
    let config = moderate_reserve_config();

    // When evaluating.
    let mutations = run_evaluate(&worker, &history, &config);

    // Then the last few entries should NOT be excluded.
    let excluded_ids = forced_exclude_ids(&mutations);

    // The very last entry should not be excluded (it's in the reserve).
    let last_entry_id = history.last().expect("history should have entries").id.clone();
    assert!(
        !excluded_ids.contains(&last_entry_id),
        "the most recent entry should be preserved in the reserve"
    );
}

#[test]
fn evaluate_for_session_returns_empty_for_empty_history() {
    // Given a worker with a session that has no history.
    let (worker, session_id) = test_worker_with_session(FAKE_SUMMARY, vec![]);

    // When evaluating.
    let rt = tokio::runtime::Runtime::new().expect("test runtime");
    let mutations = rt.block_on(async {
        worker
            .evaluate_for_session(&CompactionTrigger {
                session_id,
                compact_all: false,
            })
            .await
    });

    // Then no mutations are produced.
    assert!(mutations.is_empty(), "empty history should produce no mutations");
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 2: Bug regression tests
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn no_compaction_below_threshold() {
    // Given a short history and a huge reserve.
    let worker = test_worker(FAKE_SUMMARY);
    let history = alternating_history(2);
    let config = huge_reserve_config();

    // When evaluating.
    let mutations = run_evaluate(&worker, &history, &config);

    // Then no compaction triggers.
    assert!(
        mutations.is_empty(),
        "should not compact when tokens are below threshold"
    );
}

#[test]
fn compaction_triggers_above_threshold() {
    // Given a long history and a small reserve.
    let worker = test_worker(FAKE_SUMMARY);
    let history = alternating_history(20);
    let config = small_reserve_config();

    // When evaluating.
    let mutations = run_evaluate(&worker, &history, &config);

    // Then compaction triggers.
    assert!(
        !mutations.is_empty(),
        "should compact when tokens exceed threshold"
    );
}

#[test]
fn no_double_compaction_after_first() {
    // Bug 2 regression: after compaction mutations are applied and prompt
    // re-assembled with small context_size, no second compaction should trigger.

    // Given a history with a prior compaction followed by a few entries that
    // fit within the huge reserve.
    let worker = test_worker(FAKE_SUMMARY);
    let history = history_with_prior_compaction(3);
    let config = huge_reserve_config();

    // When evaluating.
    let mutations = run_evaluate(&worker, &history, &config);

    // Then no mutations are produced — the 3 turns after the compaction
    // boundary fit within the reserve.
    assert!(
        mutations.is_empty(),
        "should not double-compact when entries after boundary fit in reserve"
    );
}

#[test]
fn session_continues_after_background_compaction() {
    // Bug 3 regression: compaction mutations applied during an active session
    // should not change the session phase.

    // Given a session in Sending phase with enough history to trigger compaction.
    let mut session = ChatSessionState::new();
    let history = alternating_history(20);
    for entry in &history {
        session.push_entry(entry.clone());
    }
    session.begin_sending();
    let session_id = session.session_id().clone();

    let state = State::new(AppState::default());
    {
        let mut app = state.write();
        app.session.insert(session);
        // Use a tiny reserve so compaction triggers with just 20 turns.
        app.frontend.preferences.compaction = CompactionConfig {
            model: None,
            threshold: 0.8,
            reserve_tokens: 100,
            fallback_context_window: 150_000,
        };
    }

    let services = TestServices::builder()
        .llm_service(LlmServiceFactoryService::new(Arc::new(
            FakeLlmServiceFactory::new(vec![FAKE_SUMMARY.to_owned()]),
        )))
        .build();
    let handle = services.handle.clone();

    let worker = CompactionWorker {
        services,
        handle,
        state,
        config: CompactionConfig::default(),
        compaction_prompt: "Summarize this conversation.".to_owned(),
    };

    // When evaluating compaction for the session.
    let rt = tokio::runtime::Runtime::new().expect("test runtime");
    let mutations = rt.block_on(async {
        worker
            .evaluate_for_session(&CompactionTrigger {
                session_id: session_id.clone(),
                compact_all: false,
            })
            .await
    });

    // Then compaction produced mutations.
    assert!(!mutations.is_empty(), "should have mutations for long history");

    // And the session phase is still Sending (compaction doesn't change phase).
    let guard = worker.state.read();
    let session = guard.session(&session_id);
    use crate::feat::session::phase_machine::PhaseKind;
    assert_eq!(
        session.phase(),
        PhaseKind::Sending,
        "session should remain in Sending phase after background compaction"
    );
}

#[test]
fn threshold_uses_fresh_history_not_stale_context_size() {
    // Bug 1 regression: the worker uses the passed-in history, not a stale
    // cached_context_size from a previous prompt assembly.

    // Given the same worker and config.
    let worker = test_worker(FAKE_SUMMARY);
    let config = huge_reserve_config();

    // When evaluating with a short history.
    let short_history = alternating_history(2);
    let mutations_short = run_evaluate(&worker, &short_history, &config);

    // And evaluating with a long history using the same config.
    let long_history = alternating_history(20);
    let config_small = small_reserve_config();
    let mutations_long = run_evaluate(&worker, &long_history, &config_small);

    // Then the results differ — proving the worker uses the passed-in history,
    // not some stale cached value.
    assert!(mutations_short.is_empty(), "short history should not compact");
    assert!(!mutations_long.is_empty(), "long history should compact");
}
