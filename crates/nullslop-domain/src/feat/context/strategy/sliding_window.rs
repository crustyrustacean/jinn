//! Sliding window strategy — sends only the last N entries from history.
//!
//! This strategy limits context to a configurable window of the most recent
//! entries, dropping older ones. Pinned entries are always included, even if
//! they fall outside the window, at their original positions. It uses
//! [`entries_to_messages`] internally and produces no system prompt.

use async_trait::async_trait;
use error_stack::Report;

use crate::protocol::{ChatEntry, entries_to_messages};

use super::turn_grouping::{Turn, group_into_turns};

use super::types::{AssembledPrompt, AssemblyContext, PromptAssembly, PromptAssemblyError};

/// A sliding window strategy that sends only the last `window_size` entries.
pub struct SlidingWindowStrategy {
    /// Maximum number of history entries to include.
    window_size: usize,
}

impl SlidingWindowStrategy {
    /// Create a new sliding window strategy with the given window size.
    #[must_use]
    pub fn new(window_size: usize) -> Self {
        Self { window_size }
    }
}

#[async_trait]
impl PromptAssembly for SlidingWindowStrategy {
    async fn assemble(
        &self,
        context: &AssemblyContext<'_>,
    ) -> Result<AssembledPrompt, Report<PromptAssemblyError>> {
        // Group history into atomic turns.
        let turns = group_into_turns(context.history);

        // Walk turns newest → oldest, snapping outward at the window boundary.
        let mut entries_included = 0usize;
        let mut included_turns: Vec<&Turn> = Vec::new();

        for turn in turns.iter().rev() {
            if turn.is_pinned() {
                included_turns.push(turn);
                entries_included += turn.entry_count();
                continue;
            }

            if entries_included >= self.window_size {
                // Window is full. Stop accumulating unpinned turns.
                continue;
            }

            included_turns.push(turn);
            entries_included += turn.entry_count();
        }

        // Reverse to chronological order.
        included_turns.reverse();

        // Flatten to entries.
        let window: Vec<ChatEntry> = included_turns
            .into_iter()
            .flat_map(|t| t.entries().cloned().collect::<Vec<_>>())
            .collect();

        let messages = entries_to_messages(&window);
        Ok(AssembledPrompt {
            system_prompt: None,
            messages,
        })
    }

    fn name(&self) -> &'static str {
        "sliding_window"
    }
}
