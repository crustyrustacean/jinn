//! Passthrough strategy — sends all entries with no filtering or system prompt.
//!
//! This is the simplest strategy and produces identical behavior to the
//! original direct conversion. It uses [`entries_to_messages`] internally.

use async_trait::async_trait;
use error_stack::Report;

use crate::protocol::entries_to_messages;

use super::types::{AssembledPrompt, AssemblyContext, PromptAssembly, PromptAssemblyError};

/// A passthrough strategy that sends all entries unchanged.
///
/// No system prompt, no filtering. Equivalent to the original `entries_to_messages`
/// conversion that was done inline in the message queue handler.
pub struct PassthroughStrategy;

#[async_trait]
impl PromptAssembly for PassthroughStrategy {
    async fn assemble(
        &self,
        context: &AssemblyContext<'_>,
    ) -> Result<AssembledPrompt, Report<PromptAssemblyError>> {
        let messages = entries_to_messages(context.history);
        Ok(AssembledPrompt {
            system_prompt: None,
            messages,
        })
    }

    fn name(&self) -> &'static str {
        "passthrough"
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use crate::protocol::{ChatEntry, SessionId};

    use super::*;

    fn test_context<'a>(
        history: &'a [ChatEntry],
        session_id: &'a SessionId,
    ) -> AssemblyContext<'a> {
        AssemblyContext {
            history,
            tools: &[],
            model_name: "test-model",
            session_id,
            budget_offset: 0,
        }
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn passthrough_converts_all_entries() {
        // Given a history with user and assistant entries.
        let history = vec![
            ChatEntry::user("hello"),
            ChatEntry::assistant("hi there"),
            ChatEntry::user("how are you?"),
        ];

        // When assembling with passthrough strategy.
        let strategy = PassthroughStrategy;
        let session_id = SessionId::new();
        let context = test_context(&history, &session_id);
        let result = strategy.assemble(&context).await.expect("assemble");

        // Then all entries are converted and there is no system prompt.
        assert!(result.system_prompt.is_none());
        assert_eq!(result.messages.len(), 3);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn passthrough_empty_history() {
        // Given empty history.
        let history: Vec<ChatEntry> = vec![];

        // When assembling with passthrough strategy.
        let strategy = PassthroughStrategy;
        let session_id = SessionId::new();
        let context = test_context(&history, &session_id);
        let result = strategy.assemble(&context).await.expect("assemble");

        // Then no messages are produced.
        assert!(result.messages.is_empty());
        assert!(result.system_prompt.is_none());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn passthrough_name() {
        // Given a passthrough strategy.
        let strategy = PassthroughStrategy;

        // Then its name is "passthrough".
        assert_eq!(strategy.name(), "passthrough");
    }
}
