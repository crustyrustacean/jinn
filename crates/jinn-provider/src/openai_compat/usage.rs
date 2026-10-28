//! The deferred terminal `Done` event and the usage accounting attached to it.
//!
//! Most providers send `finish_reason` and `usage` in the same SSE chunk.
//! OpenRouter, with experimental metadata enabled, does not: it sends the
//! finish reason first and the usage block in a later chunk. Emitting `Done`
//! at `finish_reason` would therefore report a response with no token counts
//! and no cost, which is worse than reporting nothing.
//!
//! So the terminal event is buffered. A finish reason parks a [`PendingDone`];
//! every subsequent chunk may attach usage to it; the `[DONE]` sentinel
//! flushes it. A provider that sends both at once gets the same result — the
//! buffer is created and enriched within a single chunk — which is why this
//! is not a special case but the only path.
//!
//! Two rules make the buffering safe, and both are enforced here rather than
//! at the call site. Usage only ever *replaces* the pending usage, so a late
//! chunk with no `usage` block cannot erase counts already collected. And
//! [`PendingDone::take`] removes the buffer as it hands it over, so a second
//! `[DONE]` cannot emit the same `Done` twice.

use crate::stream_event::{StopReason, StreamUsage};

/// Why the stream stopped, held until `[DONE]` releases it.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct PendingDone {
    /// Why the stream stopped, taken from `finish_reason`.
    pub(super) stop_reason: StopReason,
    /// Usage collected so far. `None` until a chunk carries a `usage` block,
    /// and left `None` if the provider never sends one.
    pub(super) usage: Option<StreamUsage>,
    /// The model id the provider echoed on its chunks, captured by the
    /// buffer and stamped in when the event is released. `None` when the
    /// provider never sent one.
    pub(super) model: Option<String>,
}

/// The buffered terminal event, plus the enrichment it accumulates.
#[derive(Debug, Default)]
pub(super) struct PendingDoneBuffer {
    /// The buffered event, if a finish reason has been seen.
    pending: Option<PendingDone>,
    /// The model the provider echoes on its chunks, captured from the first
    /// chunk that carries a non-empty one. Held on the buffer rather than the
    /// pending event so the echo is collected even on chunks that arrive
    /// before any finish reason parks a `Done`.
    seen_model: Option<String>,
}

impl PendingDoneBuffer {
    /// Records a finish reason, if nothing is pending and the stream is live.
    ///
    /// A second finish reason in a later chunk is ignored: OpenRouter sends
    /// the same one twice, and honouring the second would reset the usage
    /// already collected. Returns `false` when a buffer already exists, which
    /// is the caller's signal that this finish reason changed nothing.
    pub(super) fn park(&mut self, stop_reason: StopReason) -> bool {
        if self.pending.is_some() {
            return false;
        }
        self.pending = Some(PendingDone {
            stop_reason,
            usage: None,
            model: self.seen_model.clone(),
        });
        true
    }

    /// Whether a finish reason is waiting to be flushed.
    pub(super) fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Attaches model and usage parsed from `chunk`, when the chunk carries
    /// them.
    ///
    /// The model read happens before the pending check: a provider echoes its
    /// model on every chunk, including the first, so the capture must run even
    /// while nothing is parked. The first non-empty id wins and later chunks
    /// cannot replace it. A chunk with no `usage` block leaves whatever usage
    /// was collected untouched, so an interleaved content chunk cannot blank
    /// the accounting.
    pub(super) fn enrich_from(&mut self, chunk: &serde_json::Value) {
        if let Some(model) = chunk
            .get("model")
            .and_then(serde_json::Value::as_str)
            .filter(|m| !m.is_empty())
            && self.seen_model.is_none()
        {
            self.seen_model = Some(model.to_owned());
        }
        let Some(pending) = &mut self.pending else {
            return;
        };
        let Some(usage_val) = chunk.get("usage") else {
            return;
        };
        pending.usage = Some(parse_usage(usage_val));
    }

    /// Removes and returns the buffered event, if there is one.
    pub(super) fn take(&mut self) -> Option<PendingDone> {
        let mut pending = self.pending.take()?;
        pending.model = self.seen_model.take();
        Some(pending)
    }
}

/// Reads a provider's `usage` block into the shape the rest of jinn uses.
///
/// `cost` and `cached_tokens` are OpenRouter and OpenAI extensions
/// respectively; a provider that omits them gets `None` rather than a zero,
/// so "not reported" stays distinguishable from "reported as free".
fn parse_usage(usage_val: &serde_json::Value) -> StreamUsage {
    StreamUsage {
        prompt_tokens: usage_val
            .get("prompt_tokens")
            .and_then(serde_json::Value::as_u64),
        completion_tokens: usage_val
            .get("completion_tokens")
            .and_then(serde_json::Value::as_u64),
        cost: usage_val.get("cost").and_then(serde_json::Value::as_f64),
        cached_tokens: usage_val
            .get("prompt_tokens_details")
            .and_then(|d| d.get("cached_tokens"))
            .and_then(serde_json::Value::as_u64),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, reason = "test code")]
    use super::*;

    #[rstest::rstest]
    fn parking_records_the_stop_reason_with_no_usage() {
        // Given an empty buffer.
        let mut buffer = PendingDoneBuffer::default();

        // When a finish reason is parked.
        assert!(buffer.park(StopReason::EndTurn));

        // Then it is pending, carrying the reason and no usage yet.
        assert!(buffer.is_pending());
        let pending = buffer.take().expect("a pending event");
        assert_eq!(pending.stop_reason, StopReason::EndTurn);
        assert_eq!(pending.usage, None);
    }

    #[rstest::rstest]
    fn a_second_finish_reason_does_not_replace_the_first() {
        // Given a buffer already holding one finish reason, enriched.
        let mut buffer = PendingDoneBuffer::default();
        buffer.park(StopReason::ToolUse);
        buffer.enrich_from(&serde_json::json!({"usage": {"prompt_tokens": 10}}));

        // When OpenRouter's repeat of the same finish reason arrives.
        assert!(!buffer.park(StopReason::EndTurn));

        // Then the original reason and its usage both survive.
        let pending = buffer.take().expect("a pending event");
        assert_eq!(pending.stop_reason, StopReason::ToolUse);
        assert_eq!(
            pending.usage.expect("usage retained").prompt_tokens,
            Some(10)
        );
    }

    #[rstest::rstest]
    fn a_usage_only_chunk_enriches_a_pending_event() {
        // Given a buffer holding a parked finish reason.
        let mut buffer = PendingDoneBuffer::default();
        buffer.park(StopReason::EndTurn);

        // When a later chunk carries usage and nothing else.
        buffer.enrich_from(&serde_json::json!({
            "id": "x",
            "usage": {"prompt_tokens": 100, "completion_tokens": 50, "cost": 0.5}
        }));

        // Then the buffered event carries that usage.
        let pending = buffer.take().expect("a pending event");
        let usage = pending.usage.expect("usage attached");
        assert_eq!(usage.prompt_tokens, Some(100));
        assert_eq!(usage.completion_tokens, Some(50));
        assert_eq!(usage.cost, Some(0.5));
    }

    #[rstest::rstest]
    fn a_chunk_without_usage_leaves_collected_counts_intact() {
        // Given a buffer whose usage has already been collected.
        let mut buffer = PendingDoneBuffer::default();
        buffer.park(StopReason::EndTurn);
        buffer.enrich_from(&serde_json::json!({"usage": {"prompt_tokens": 42}}));

        // When an interleaved content chunk with no `usage` block arrives.
        buffer.enrich_from(&serde_json::json!({
            "id": "x",
            "choices": [{"delta": {"content": "more text"}}]
        }));

        // Then the counts are not erased.
        let pending = buffer.take().expect("a pending event");
        assert_eq!(
            pending.usage.expect("usage retained").prompt_tokens,
            Some(42)
        );
    }

    #[rstest::rstest]
    fn a_later_usage_block_replaces_an_earlier_one() {
        // Given a buffer enriched once.
        let mut buffer = PendingDoneBuffer::default();
        buffer.park(StopReason::EndTurn);
        buffer.enrich_from(&serde_json::json!({"usage": {"prompt_tokens": 1}}));

        // When a second usage block arrives with a fuller accounting.
        buffer.enrich_from(&serde_json::json!({
            "usage": {
                "prompt_tokens": 1000,
                "completion_tokens": 50,
                "prompt_tokens_details": {"cached_tokens": 400}
            }
        }));

        // Then the later block wins rather than the first being kept.
        let pending = buffer.take().expect("a pending event");
        let usage = pending.usage.expect("usage attached");
        assert_eq!(usage.prompt_tokens, Some(1000));
        assert_eq!(usage.cached_tokens, Some(400));
    }

    #[rstest::rstest]
    fn cached_tokens_read_from_prompt_tokens_details() {
        // Given a usage block carrying OpenAI's cached-token detail.
        let mut buffer = PendingDoneBuffer::default();
        buffer.park(StopReason::EndTurn);

        // When it is absorbed.
        buffer.enrich_from(&serde_json::json!({
            "usage": {"prompt_tokens": 1000, "prompt_tokens_details": {"cached_tokens": 250}}
        }));

        // Then the cache hits are reported.
        let pending = buffer.take().expect("a pending event");
        assert_eq!(
            pending.usage.expect("usage attached").cached_tokens,
            Some(250)
        );
    }

    #[rstest::rstest]
    fn cost_is_none_when_the_provider_does_not_report_it() {
        // Given a usage block from a provider with no cost extension.
        let mut buffer = PendingDoneBuffer::default();
        buffer.park(StopReason::EndTurn);

        // When it is absorbed.
        buffer.enrich_from(&serde_json::json!({
            "usage": {"prompt_tokens": 100, "completion_tokens": 50}
        }));

        // Then cost is absent rather than zero.
        let pending = buffer.take().expect("a pending event");
        let usage = pending.usage.expect("usage attached");
        assert_eq!(usage.cost, None);
        assert_eq!(usage.cached_tokens, None);
    }

    #[rstest::rstest]
    fn enriching_without_a_parked_event_is_ignored() {
        // Given an empty buffer.
        let mut buffer = PendingDoneBuffer::default();

        // When a usage block arrives before any finish reason.
        buffer.enrich_from(&serde_json::json!({"usage": {"prompt_tokens": 99}}));

        // Then nothing is pending; early usage is not a completion signal.
        assert!(!buffer.is_pending());
        assert!(buffer.take().is_none());
    }

    #[rstest::rstest]
    fn taking_twice_yields_the_event_once() {
        // Given a buffer holding a parked finish reason.
        let mut buffer = PendingDoneBuffer::default();
        buffer.park(StopReason::EndTurn);

        // When it is taken twice.
        let first = buffer.take();
        let second = buffer.take();

        // Then only the first take returns the event.
        assert!(first.is_some());
        assert!(second.is_none());
    }

    #[rstest::rstest]
    fn sse_captures_echoed_model() {
        // Given a parser fed a chunk carrying the provider's model echo.
        let mut buffer = PendingDoneBuffer::default();

        // When a chunk with a top-level `model` field arrives.
        buffer.enrich_from(&serde_json::json!({
            "id": "x",
            "model": "anthropic/claude-sonnet-4.5",
            "choices": [{"delta": {"content": "hi"}}]
        }));

        // Then parking a finish reason stamps the captured model into the
        // pending event.
        buffer.park(StopReason::EndTurn);
        let pending = buffer.take().expect("a pending event");
        assert_eq!(
            pending.model.as_deref(),
            Some("anthropic/claude-sonnet-4.5")
        );
    }

    #[rstest::rstest]
    fn first_reported_model_survives_later_chunks() {
        // Given a buffer that already captured a model echo.
        let mut buffer = PendingDoneBuffer::default();
        buffer.enrich_from(&serde_json::json!({"model": "anthropic/claude-sonnet-4.5"}));

        // When a later chunk reports a different model.
        buffer.enrich_from(&serde_json::json!({"model": "openai/gpt-5.1"}));

        // Then the first capture wins rather than being replaced.
        buffer.park(StopReason::EndTurn);
        let pending = buffer.take().expect("a pending event");
        assert_eq!(
            pending.model.as_deref(),
            Some("anthropic/claude-sonnet-4.5")
        );
    }

    #[rstest::rstest]
    fn model_captured_before_finish_reason_parks_done() {
        // Given a buffer fed only first-chunk echoes, with no finish reason yet.
        let mut buffer = PendingDoneBuffer::default();
        buffer.enrich_from(&serde_json::json!({"model": "openrouter/auto"}));

        // Then nothing is pending, but the echo is held for later parking.
        assert!(!buffer.is_pending());
        buffer.park(StopReason::ToolUse);
        let pending = buffer.take().expect("a pending event");
        assert_eq!(pending.model.as_deref(), Some("openrouter/auto"));
    }

    #[rstest::rstest]
    fn done_without_any_model_echo_leaves_model_none() {
        // Given a buffer fed chunks that never carry a `model` field.
        let mut buffer = PendingDoneBuffer::default();
        buffer.enrich_from(&serde_json::json!({
            "choices": [{"delta": {"content": "hi"}}]
        }));

        // When the stream finishes and the event is released.
        buffer.park(StopReason::EndTurn);
        let pending = buffer.take().expect("a pending event");

        // Then the released event carries no model.
        assert_eq!(pending.model, None);
    }
}
