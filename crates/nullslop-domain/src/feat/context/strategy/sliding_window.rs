//! Sliding window strategy — sends only the last N entries from history.
//!
//! This strategy limits context to a configurable window of the most recent
//! entries, dropping older ones. Pinned entries are always included, even if
//! they fall outside the window, at their original positions. It uses
//! [`entries_to_messages`] internally and produces no system prompt.

use async_trait::async_trait;
use error_stack::Report;

use crate::protocol::{ChatEntry, entries_to_messages};

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
        let window_start = context.history.len().saturating_sub(self.window_size);

        // Include entries that are in the window OR are pinned.
        // Pinned entries outside the window are kept at their original positions.
        let window: Vec<ChatEntry> = context
            .history
            .iter()
            .enumerate()
            .filter(|(i, entry)| *i >= window_start || entry.is_pinned())
            .map(|(_, entry)| entry.clone())
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
