//! Turn grouping — partitions chat history into atomic units for eviction-safe assembly.
//!
//! A [`Turn`] groups related entries that must never be split by prompt assembly
//! strategies. The primary concern is tool-loop integrity: an `Assistant` with
//! `tool_calls` must always be accompanied by its matching `ToolResult` entries.
//!
//! # Turn types
//!
//! - **Standalone**: any single entry that is not part of a tool loop (User, System,
//!   plain Assistant, etc.)
//! - **Tool-loop**: `Assistant` followed by `ToolCall(s)` and `ToolResult(s)` —
//!   all grouped into one turn

use crate::feat::context::strategy::token_estimator::{TokenEstimator, estimate_entry_tokens};
use crate::protocol::{ChatEntry, ChatEntryKind};

/// A group of chat entries that must be kept together during prompt assembly.
///
/// Turns are the atomic unit of eviction. Strategies walk turns instead of
/// individual entries, guaranteeing that tool-call pairs are never split.
#[derive(Debug, Clone)]
pub struct Turn(Vec<ChatEntry>);

impl Turn {
    /// Token cost of this turn — sum of all entries' token estimates.
    pub fn token_cost(&self, estimator: &dyn TokenEstimator) -> usize {
        self.0
            .iter()
            .map(|e| estimate_entry_tokens(estimator, e))
            .sum()
    }

    /// Whether any entry in this turn is pinned.
    pub fn is_pinned(&self) -> bool {
        self.0.iter().any(ChatEntry::is_pinned)
    }

    /// Number of individual entries in this turn.
    pub fn entry_count(&self) -> usize {
        self.0.len()
    }

    /// Iterator over entries in this turn.
    pub fn entries(&self) -> impl Iterator<Item = &ChatEntry> {
        self.0.iter()
    }

    /// Whether this turn contains any ToolCall or ToolResult entries.
    pub fn is_tool_loop(&self) -> bool {
        self.0.iter().any(|e| {
            matches!(
                e.kind,
                ChatEntryKind::ToolCall { .. } | ChatEntryKind::ToolResult { .. }
            )
        })
    }
}

/// Groups chat history entries into atomic turns.
///
/// Walks history forward in a single pass. The rules:
///
/// - `Assistant` followed immediately by a `ToolCall` starts a **tool-loop turn**
///   that absorbs all subsequent `ToolCall` and `ToolResult` entries.
/// - The tool-loop turn ends when the next entry is neither `ToolCall` nor
///   `ToolResult`.
/// - A non-tool `Assistant` (not followed by `ToolCall`) is a standalone turn.
/// - All other entries (`User`, `System`, `Error`, etc.) are standalone turns.
/// - Orphaned `ToolCall` or `ToolResult` entries (no preceding `Assistant`) become
///   standalone turns — they don't panic but shouldn't appear in normal operation.
pub fn group_into_turns(history: &[ChatEntry]) -> Vec<Turn> {
    let mut turns = Vec::new();
    let mut current_turn: Option<Vec<ChatEntry>> = None;

    for (i, entry) in history.iter().enumerate() {
        match &entry.kind {
            ChatEntryKind::ToolCall { .. } | ChatEntryKind::ToolResult { .. } => {
                // Part of a tool loop — append to current turn.
                // If no current turn (orphaned), start a new standalone turn.
                if let Some(ref mut turn) = current_turn {
                    turn.push(entry.clone());
                } else {
                    turns.push(Turn(vec![entry.clone()]));
                }
            }
            ChatEntryKind::Assistant(..) => {
                // Check if this assistant starts a tool loop.
                let next_is_tool_call = history
                    .get(i + 1)
                    .is_some_and(|e| matches!(e.kind, ChatEntryKind::ToolCall { .. }));

                if next_is_tool_call {
                    // Finalize any in-progress turn.
                    if let Some(entries) = current_turn.take() {
                        turns.push(Turn(entries));
                    }
                    // Start a new tool-loop turn.
                    current_turn = Some(vec![entry.clone()]);
                } else {
                    // Standalone assistant — finalize previous turn, emit this one.
                    if let Some(entries) = current_turn.take() {
                        turns.push(Turn(entries));
                    }
                    turns.push(Turn(vec![entry.clone()]));
                }
            }
            _ => {
                // Any non-tool entry — finalize previous turn, emit standalone.
                if let Some(entries) = current_turn.take() {
                    turns.push(Turn(entries));
                }
                turns.push(Turn(vec![entry.clone()]));
            }
        }
    }

    // Finalize any in-progress turn.
    if let Some(entries) = current_turn {
        turns.push(Turn(entries));
    }

    turns
}
