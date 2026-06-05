//! Regex-based auto-prune worker.
//!
//! Matches tool calls by regex pattern and prunes all but the most recent
//! `keep_last` matching call+result pairs. Rules are configured via
//! `[[auto_prune.regex.rules]]` in `jinn.toml`.
//!
//! Regex patterns are compiled once at construction time via
//! [`RegexAutoPruneWorker::from_config`] and never recompiled during evaluation.

use crate::feat::history_worker::worker_trait::HistoryWorker;
use crate::feat::preferences_actor::user_preferences::RegexAutoPruneConfig;
use crate::feat::session::chat_entry::{ChangeSource, ChatEntry, ChatEntryId, ChatEntryKind, ContextOverride};
use crate::feat::session::history_mutation::HistoryMutation;
use crate::protocol::SessionId;

/// A compiled regex prune rule (non-serializable runtime type).
///
/// Created from [`RegexPruneRule`](crate::feat::preferences_actor::user_preferences::RegexPruneRule)
/// during worker construction. The regex is compiled exactly once.
struct CompiledRegexRule {
    /// The compiled regex pattern.
    regex: regex::Regex,
    /// Tool name to filter by (e.g., "bash").
    tool_name: String,
    /// Number of most recent matching pairs to keep (minimum 1).
    keep_last: usize,
}

/// Regex-based auto-prune worker.
///
/// Holds a set of compiled regex rules. On each `HistoryAppended` event,
/// scans history for matching tool calls and prunes older ones by emitting
/// `SetContextOverride::ForcedExclude` for both the call and its result.
#[derive(Clone)]
pub struct RegexAutoPruneWorker {
    /// Compiled regex rules (empty if disabled or no rules configured).
    rules: Vec<CompiledRegexRule>,
}

impl RegexAutoPruneWorker {
    /// Construct a worker from config, compiling all regex patterns once.
    ///
    /// Returns `Err(regex::Error)` if any pattern is invalid.
    /// Clamps `keep_last` to a minimum of 1.
    /// Returns an empty worker if disabled or no rules configured.
    ///
    /// # Errors
    ///
    /// Returns `regex::Error` if any pattern string fails to compile.
    pub fn from_config(config: &RegexAutoPruneConfig) -> Result<Self, regex::Error> {
        if !config.enabled || config.rules.is_empty() {
            return Ok(Self { rules: Vec::new() });
        }

        let mut compiled = Vec::with_capacity(config.rules.len());
        for rule in &config.rules {
            let regex = regex::Regex::new(&rule.pattern)?;
            compiled.push(CompiledRegexRule {
                regex,
                tool_name: rule.tool_name.clone(),
                keep_last: rule.keep_last.max(1),
            });
        }

        Ok(Self { rules: compiled })
    }
}

// Manual Clone impl because regex::Regex doesn't derive Clone.
// (Actually it does implement Clone, but the struct isn't Clone by default.)
impl Clone for CompiledRegexRule {
    fn clone(&self) -> Self {
        Self {
            regex: self.regex.clone(),
            tool_name: self.tool_name.clone(),
            keep_last: self.keep_last,
        }
    }
}

/// Walk forward from a ToolCall to find its matching ToolResult by tool call ID.
///
/// Returns `None` if no matching result exists (pending/orphaned call).
fn find_matching_result(
    history: &[ChatEntry],
    call_idx: usize,
    tool_call_id: &str,
) -> Option<ChatEntryId> {
    // ToolResults appear after their ToolCall, so scan forward only.
    for entry in history.iter().skip(call_idx + 1) {
        if let ChatEntryKind::ToolResult { id, .. } = &entry.kind
            && id == tool_call_id
        {
            return Some(entry.id.clone());
        }
    }
    // No matching result found — the call is still pending or orphaned.
    None
}

/// Scan history for ToolCalls matching a single regex rule and collect
/// (call_entry_id, result_entry_id) pairs.
///
/// Matches regardless of exclusion status — already-excluded entries still
/// count toward `keep_last` positioning so that the "most recent N" window
/// is stable regardless of prior pruning.
fn collect_matching_pairs(
    history: &[ChatEntry],
    rule: &CompiledRegexRule,
) -> Vec<(ChatEntryId, ChatEntryId)> {
    let mut matched_pairs: Vec<(ChatEntryId, ChatEntryId)> = Vec::new();

    for (i, entry) in history.iter().enumerate() {
        // Only interested in ToolCall entries matching the rule's tool_name.
        let tool_call_id = match &entry.kind {
            ChatEntryKind::ToolCall { id, name, .. } if name == &rule.tool_name => id.clone(),
            _ => continue,
        };

        // Run regex against the full text() output: "{name}: {arguments}".
        // Match regardless of current exclusion status so that already-excluded
        // entries still count toward keep_last positioning.
        let text = entry.text();
        let matched = rule.regex.is_match(&text);
        tracing::info!(
            entry_id = %entry.id,
            text = %text,
            matched,
            "regex match attempt"
        );
        if !matched {
            continue;
        }

        // Walk forward to find the ToolResult for this matching call.
        // If none found (pending/orphaned), skip — incomplete pairs don't
        // count toward keep_last positioning.
        if let Some(result_id) = find_matching_result(history, i, &tool_call_id) {
            tracing::info!(
                call_id = %entry.id,
                result_id = %result_id,
                "matched pair"
            );
            matched_pairs.push((entry.id.clone(), result_id));
        } else {
            tracing::warn!(call_id = %entry.id, "no matching result found");
        }
    }

    matched_pairs
}

/// For each rule, prune all but the last `keep_last` matching pairs.
///
/// Pairs are in history order (oldest first), so `.take()` selects the
/// oldest pairs to prune. Only emits mutations for entries not already excluded.
fn build_prune_mutations(
    history: &[ChatEntry],
    rules: &[CompiledRegexRule],
    worker_name: &str,
) -> Vec<HistoryMutation> {
    let mut mutations = Vec::new();

    for rule in rules {
        let matched_pairs = collect_matching_pairs(history, rule);

        tracing::info!(
            rule = %rule.regex,
            matched_count = matched_pairs.len(),
            keep_last = rule.keep_last,
            "pruning decision",
        );

        if matched_pairs.len() <= rule.keep_last {
            continue;
        }

        // Pairs are oldest-first. Prune the oldest ones beyond keep_last.
        let prune_count = matched_pairs.len() - rule.keep_last;
        let rule_ident = rule.regex.as_str();
        for (idx, (call_id, result_id)) in matched_pairs.iter().take(prune_count).enumerate() {
            // Only emit mutations for entries not already excluded.
            let call_already_excluded = history
                .iter()
                .any(|e| e.id == *call_id && e.context_override() == ContextOverride::ForcedExclude);
            let result_already_excluded = history.iter().any(|e| {
                e.id == *result_id && e.context_override() == ContextOverride::ForcedExclude
            });

            if !call_already_excluded {
                tracing::info!(
                    rule = %rule_ident,
                    pair_index = idx,
                    entry_id = %call_id,
                    "emitting ForcedExclude for call"
                );
                mutations.push(HistoryMutation::SetContextOverride {
                    entry_id: call_id.clone(),
                    value: ContextOverride::ForcedExclude,
                    source: ChangeSource::Worker {
                        name: worker_name.to_owned(),
                    },
                });
            }
            if !result_already_excluded {
                tracing::info!(
                    rule = %rule_ident,
                    pair_index = idx,
                    entry_id = %result_id,
                    "emitting ForcedExclude for result"
                );
                mutations.push(HistoryMutation::SetContextOverride {
                    entry_id: result_id.clone(),
                    value: ContextOverride::ForcedExclude,
                    source: ChangeSource::Worker {
                        name: worker_name.to_owned(),
                    },
                });
            }
        }
    }

    mutations
}

#[async_trait::async_trait]
impl HistoryWorker for RegexAutoPruneWorker {
    #[allow(clippy::unnecessary_literal_bound)]
    fn name(&self) -> &str {
        "auto-prune-regex"
    }

    async fn evaluate(
        &self,
        _session_id: &SessionId,
        history: std::sync::Arc<[ChatEntry]>,
    ) -> Vec<HistoryMutation> {
        let mutations = build_prune_mutations(&history, &self.rules, self.name());

        tracing::info!(total_mutations = mutations.len(), "regex worker done");
        mutations
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

    use super::*;
    use crate::feat::preferences_actor::user_preferences::{RegexAutoPruneConfig, RegexPruneRule};
    use crate::feat::session::chat_entry::{ChangeSource, ChatEntry, ContextOverride};
    use crate::feat::session::tool_result_status::ToolResultStatus;
    use crate::protocol::SessionId;

    /// Helper: create a bash ToolCall + ToolResult pair.
    fn bash_call_result(call_id: &str, command: &str, output: &str) -> [ChatEntry; 2] {
        [
            ChatEntry::tool_call(call_id, "bash", format!(r#"{{"command": "{command}"}}"#)),
            ChatEntry::tool_result(call_id, "bash", output, ToolResultStatus::Success),
        ]
    }

    /// Helper: create a read ToolCall + ToolResult pair.
    fn read_call_result(call_id: &str, path: &str, output: &str) -> [ChatEntry; 2] {
        [
            ChatEntry::tool_call(call_id, "read", format!(r#"{{"path": "{path}"}}"#)),
            ChatEntry::tool_result(call_id, "read", output, ToolResultStatus::Success),
        ]
    }

    /// Helper: build a worker from rules with given keep_last for "cargo check" pattern.
    fn worker_for_cargo_check(keep_last: usize) -> RegexAutoPruneWorker {
        RegexAutoPruneWorker::from_config(&RegexAutoPruneConfig {
            enabled: true,
            rules: vec![RegexPruneRule {
                pattern: "cargo check".to_owned(),
                tool_name: "bash".to_owned(),
                keep_last,
            }],
        })
        .expect("valid config")
    }

    /// Helper: evaluate a worker on a history snapshot.
    fn evaluate(worker: &RegexAutoPruneWorker, history: Vec<ChatEntry>) -> Vec<HistoryMutation> {
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let history: std::sync::Arc<[ChatEntry]> = history.into();
        rt.block_on(async { worker.evaluate(&SessionId::new(), history).await })
    }

    // --- from_config tests ---

    #[test]
    fn from_config_clamps_keep_last_to_minimum_1() {
        let worker = RegexAutoPruneWorker::from_config(&RegexAutoPruneConfig {
            enabled: true,
            rules: vec![RegexPruneRule {
                pattern: "cargo check".to_owned(),
                tool_name: "bash".to_owned(),
                keep_last: 0,
            }],
        })
        .expect("valid config");

        // Verify by checking behavior: with 1 match and keep_last clamped to 1, no pruning.
        let history = vec![
            bash_call_result("tc-1", "cargo check", "ok")[0].clone(),
            bash_call_result("tc-1", "cargo check", "ok")[1].clone(),
        ];
        let mutations = evaluate(&worker, history);
        assert!(
            mutations.is_empty(),
            "keep_last clamped to 1, single match should not prune"
        );
    }

    #[test]
    fn from_config_returns_error_for_invalid_regex() {
        let result = RegexAutoPruneWorker::from_config(&RegexAutoPruneConfig {
            enabled: true,
            rules: vec![RegexPruneRule {
                pattern: "[".to_owned(),
                tool_name: "bash".to_owned(),
                keep_last: 1,
            }],
        });
        assert!(result.is_err(), "invalid regex should return error");
    }

    #[test]
    fn from_config_disabled_returns_empty_worker() {
        let worker = RegexAutoPruneWorker::from_config(&RegexAutoPruneConfig {
            enabled: false,
            rules: vec![RegexPruneRule {
                pattern: "cargo check".to_owned(),
                tool_name: "bash".to_owned(),
                keep_last: 1,
            }],
        })
        .expect("valid config");

        let history = vec![
            bash_call_result("tc-1", "cargo check", "ok")[0].clone(),
            bash_call_result("tc-1", "cargo check", "ok")[1].clone(),
        ];
        let mutations = evaluate(&worker, history);
        assert!(
            mutations.is_empty(),
            "disabled worker should produce no mutations"
        );
    }

    // --- evaluate tests ---

    #[test]
    fn no_matching_tool_calls_produces_no_mutations() {
        let history = vec![ChatEntry::user("hello"), ChatEntry::assistant("hi")];
        let worker = worker_for_cargo_check(1);
        let mutations = evaluate(&worker, history);
        assert!(mutations.is_empty());
    }

    #[test]
    fn single_match_with_keep_last_1_produces_no_mutations() {
        let pair = bash_call_result("tc-1", "cargo check 2>&1", "all good");
        let history = vec![pair[0].clone(), pair[1].clone()];
        let worker = worker_for_cargo_check(1);
        let mutations = evaluate(&worker, history);
        assert!(mutations.is_empty());
    }

    #[test]
    fn three_matches_keep_last_1_prunes_two_oldest() {
        let mut history = Vec::new();

        // First pair
        let p1 = bash_call_result("tc-1", "cargo check", "errors: 0");
        history.push(p1[0].clone());
        history.push(p1[1].clone());

        // Second pair
        let p2 = bash_call_result("tc-2", "cargo check", "errors: 0");
        history.push(p2[0].clone());
        history.push(p2[1].clone());

        // Third pair (most recent)
        let p3 = bash_call_result("tc-3", "cargo check", "errors: 0");
        history.push(p3[0].clone());
        history.push(p3[1].clone());

        // Save entry IDs before move.
        let id_0 = history[0].id.clone();
        let id_1 = history[1].id.clone();
        let id_2 = history[2].id.clone();
        let id_3 = history[3].id.clone();
        let id_4 = history[4].id.clone();
        let id_5 = history[5].id.clone();

        let worker = worker_for_cargo_check(1);
        let mutations = evaluate(&worker, history);

        // Should prune 2 oldest pairs = 4 mutations (2 calls + 2 results).
        assert_eq!(mutations.len(), 4);

        // Verify the first two calls and their results are targeted.
        let mut excluded_ids = std::collections::HashSet::new();
        for m in &mutations {
            if let HistoryMutation::SetContextOverride { entry_id, value, .. } = m {
                assert_eq!(*value, ContextOverride::ForcedExclude);
                excluded_ids.insert(entry_id);
            }
        }

        // First two calls and first two results should be excluded.
        assert!(excluded_ids.contains(&id_0), "tc-1 call should be excluded");
        assert!(
            excluded_ids.contains(&id_1),
            "tc-1 result should be excluded"
        );
        assert!(excluded_ids.contains(&id_2), "tc-2 call should be excluded");
        assert!(
            excluded_ids.contains(&id_3),
            "tc-2 result should be excluded"
        );
        // Third pair should NOT be excluded.
        assert!(!excluded_ids.contains(&id_4), "tc-3 call should be kept");
        assert!(!excluded_ids.contains(&id_5), "tc-3 result should be kept");
    }

    #[test]
    fn three_matches_keep_last_2_prunes_one_oldest() {
        let mut history = Vec::new();

        let p1 = bash_call_result("tc-1", "cargo check", "errors: 0");
        history.push(p1[0].clone());
        history.push(p1[1].clone());

        let p2 = bash_call_result("tc-2", "cargo check", "errors: 0");
        history.push(p2[0].clone());
        history.push(p2[1].clone());

        let p3 = bash_call_result("tc-3", "cargo check", "errors: 0");
        history.push(p3[0].clone());
        history.push(p3[1].clone());

        let worker = worker_for_cargo_check(2);
        let mutations = evaluate(&worker, history);

        // Should prune 1 oldest pair = 2 mutations.
        assert_eq!(mutations.len(), 2);
    }

    #[test]
    fn keep_last_3_with_only_2_matches_produces_no_mutations() {
        let mut history = Vec::new();

        let p1 = bash_call_result("tc-1", "cargo check", "errors: 0");
        history.push(p1[0].clone());
        history.push(p1[1].clone());

        let p2 = bash_call_result("tc-2", "cargo check", "errors: 0");
        history.push(p2[0].clone());
        history.push(p2[1].clone());

        let worker = worker_for_cargo_check(3);
        let mutations = evaluate(&worker, history);
        assert!(mutations.is_empty());
    }

    #[test]
    fn already_excluded_entries_are_not_re_pruned() {
        let mut history = Vec::new();

        let p1 = bash_call_result("tc-1", "cargo check", "errors: 0");
        let mut call1 = p1[0].clone();
        call1.apply_context_override(ContextOverride::ForcedExclude, ChangeSource::Internal { label: "test".into() }); // already excluded
        history.push(call1);
        history.push(p1[1].clone());

        let p2 = bash_call_result("tc-2", "cargo check", "errors: 0");
        history.push(p2[0].clone());
        history.push(p2[1].clone());

        // Clone ID before move.
        let result1_id = history[1].id.clone();
        let worker = worker_for_cargo_check(1);
        let mutations = evaluate(&worker, history);

        // With the new logic, excluded entries still count for positioning.
        // Both pairs match, keep_last=1, so the first pair is pruned.
        // The first call is already excluded, so only the first result gets a mutation.
        assert_eq!(mutations.len(), 1);
        match &mutations[0] {
            HistoryMutation::SetContextOverride { entry_id, value, .. } => {
                assert_eq!(*entry_id, result1_id);
                assert_eq!(*value, ContextOverride::ForcedExclude);
            }
            other => panic!("expected SetContextOverride, got {other:?}"),
        }
    }

    #[test]
    fn multiple_rules_apply_independently() {
        let worker = RegexAutoPruneWorker::from_config(&RegexAutoPruneConfig {
            enabled: true,
            rules: vec![
                RegexPruneRule {
                    pattern: "cargo check".to_owned(),
                    tool_name: "bash".to_owned(),
                    keep_last: 1,
                },
                RegexPruneRule {
                    pattern: "cargo test".to_owned(),
                    tool_name: "bash".to_owned(),
                    keep_last: 1,
                },
            ],
        })
        .expect("valid config");

        let mut history = Vec::new();

        // Two cargo check calls
        let c1 = bash_call_result("tc-1", "cargo check", "ok");
        history.push(c1[0].clone());
        history.push(c1[1].clone());

        let c2 = bash_call_result("tc-2", "cargo check", "ok");
        history.push(c2[0].clone());
        history.push(c2[1].clone());

        // Two cargo test calls
        let t1 = bash_call_result("tc-3", "cargo test", "passed");
        history.push(t1[0].clone());
        history.push(t1[1].clone());

        let t2 = bash_call_result("tc-4", "cargo test", "passed");
        history.push(t2[0].clone());
        history.push(t2[1].clone());

        let mutations = evaluate(&worker, history);

        // Each rule prunes 1 oldest pair = 2 rules * 2 mutations = 4 total.
        assert_eq!(mutations.len(), 4);
    }

    #[test]
    fn rules_filter_by_tool_name() {
        let worker = RegexAutoPruneWorker::from_config(&RegexAutoPruneConfig {
            enabled: true,
            rules: vec![RegexPruneRule {
                pattern: "foo".to_owned(),
                tool_name: "bash".to_owned(),
                keep_last: 1,
            }],
        })
        .expect("valid config");

        let mut history = Vec::new();

        // "read" tool call that contains "foo" — should NOT match.
        let r1 = read_call_result("tc-1", "/foo.rs", "contents");
        history.push(r1[0].clone());
        history.push(r1[1].clone());

        let mutations = evaluate(&worker, history);
        assert!(
            mutations.is_empty(),
            "read tool should not match bash-only rule"
        );
    }

    #[test]
    fn regex_matches_against_tool_call_text() {
        let worker = RegexAutoPruneWorker::from_config(&RegexAutoPruneConfig {
            enabled: true,
            rules: vec![RegexPruneRule {
                pattern: r"bash:.*cargo check".to_owned(),
                tool_name: "bash".to_owned(),
                keep_last: 1,
            }],
        })
        .expect("valid config");

        let mut history = Vec::new();

        let p1 = bash_call_result("tc-1", "cargo check", "ok");
        history.push(p1[0].clone());
        history.push(p1[1].clone());

        let p2 = bash_call_result("tc-2", "cargo check", "ok");
        history.push(p2[0].clone());
        history.push(p2[1].clone());

        let mutations = evaluate(&worker, history);

        // Pattern matches "bash: {"command": "cargo check"}".
        assert_eq!(mutations.len(), 2, "should prune the older pair");
    }

    #[test]
    fn empty_history_produces_no_mutations() {
        let worker = worker_for_cargo_check(1);
        let mutations = evaluate(&worker, vec![]);
        assert!(mutations.is_empty());
    }

    #[test]
    fn tool_call_without_matching_result_is_skipped() {
        let history = vec![ChatEntry::tool_call(
            "tc-orphan",
            "bash",
            r#"{"command": "cargo check"}"#,
        )];

        let worker = worker_for_cargo_check(1);
        let mutations = evaluate(&worker, history);
        assert!(mutations.is_empty());
    }

    #[test]
    fn multiple_rules_same_tool_different_patterns() {
        let worker = RegexAutoPruneWorker::from_config(&RegexAutoPruneConfig {
            enabled: true,
            rules: vec![
                RegexPruneRule {
                    pattern: "cargo check".to_owned(),
                    tool_name: "bash".to_owned(),
                    keep_last: 1,
                },
                RegexPruneRule {
                    pattern: "cargo clippy".to_owned(),
                    tool_name: "bash".to_owned(),
                    keep_last: 1,
                },
            ],
        })
        .expect("valid config");

        let mut history = Vec::new();

        // Two cargo check calls
        let c1 = bash_call_result("tc-1", "cargo check", "ok");
        history.push(c1[0].clone());
        history.push(c1[1].clone());

        let c2 = bash_call_result("tc-2", "cargo check", "ok");
        history.push(c2[0].clone());
        history.push(c2[1].clone());

        // Two cargo clippy calls
        let cl1 = bash_call_result("tc-3", "cargo clippy", "ok");
        history.push(cl1[0].clone());
        history.push(cl1[1].clone());

        let cl2 = bash_call_result("tc-4", "cargo clippy", "ok");
        history.push(cl2[0].clone());
        history.push(cl2[1].clone());

        let mutations = evaluate(&worker, history);

        // Each rule prunes 1 oldest pair = 2 rules * 2 mutations = 4 total.
        assert_eq!(mutations.len(), 4);
    }
}
