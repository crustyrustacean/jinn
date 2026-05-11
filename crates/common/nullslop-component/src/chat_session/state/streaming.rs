//! Streaming methods for [`ChatSessionState`](super::ChatSessionState).
//!
//! Covers LLM token streaming and tool-call streaming.

use std::collections::HashMap;

use nullslop_protocol::{ChatEntry, ChatEntryKind};

use super::ChatSessionState;

impl ChatSessionState {
    /// Begin a new streaming response.
    ///
    /// Creates an empty `Assistant` entry, marks the session as streaming,
    /// and returns the index of the new entry.
    ///
    /// # Panics
    ///
    /// Panics if the session is already streaming. This is a programming error —
    /// the caller must ensure the previous stream has finished or been cancelled
    /// before starting a new one.
    pub fn begin_streaming(&mut self) -> usize {
        assert!(
            !self.core.is_streaming,
            "begin_streaming called while already streaming"
        );
        let entry = ChatEntry::assistant("");
        let index = self.push_entry(entry);
        self.core.streaming_entry_index = Some(index);
        self.core.is_streaming = true;
        index
    }

    /// Append a token to the streaming assistant entry.
    ///
    /// # Panics
    ///
    /// Panics if the session is not streaming. This is a programming error.
    #[expect(
        clippy::indexing_slicing,
        reason = "index comes from push_entry which always returns a valid index"
    )]
    #[expect(
        clippy::expect_used,
        reason = "streaming_entry_index invariant guaranteed by begin_streaming"
    )]
    #[expect(
        clippy::panic,
        reason = "streaming invariant violated: entry must be Assistant during active stream"
    )]
    pub fn append_stream_token<S>(&mut self, token: S)
    where
        S: AsRef<str>,
    {
        assert!(
            self.core.is_streaming,
            "append_stream_token called while not streaming"
        );
        let index = self
            .core
            .streaming_entry_index
            .expect("streaming_entry_index must be set when is_streaming");
        if let ChatEntry {
            kind: ChatEntryKind::Assistant(ref mut text),
            ..
        } = self.core.history[index]
        {
            text.push_str(token.as_ref());
        } else {
            panic!("streaming entry is not an Assistant entry");
        }
    }

    /// Mark streaming as finished (normal completion).
    pub fn finish_streaming(&mut self) {
        self.core.is_streaming = false;
        self.core.is_sending = false; // defensive: clear both on finish
        self.core.streaming_entry_index = None;
        self.core.streaming_tool_call_indices.clear();
    }

    /// Cancel streaming but keep partial text in history.
    pub fn cancel_streaming(&mut self) {
        self.core.is_streaming = false;
        self.core.is_sending = false; // defensive: clear both on cancel
        self.core.streaming_entry_index = None;
        self.core.streaming_tool_call_indices.clear();
    }

    /// Cancel streaming and drain queued messages back to the input buffer.
    ///
    /// Used when the user interrupts or switches to Normal mode during an
    /// active stream. The drained queue text is joined with newlines and
    /// replaces whatever was in the input box.
    pub fn cancel_stream_and_drain(&mut self) {
        self.cancel_streaming();
        let drained: Vec<String> = self.drain_queue().into_iter().collect();
        let drained_text = drained.join("\n");
        if !drained_text.is_empty() {
            self.chat_input_mut().replace_all(drained_text);
        }
    }

    /// Whether an LLM stream is actively producing tokens.
    pub fn is_streaming(&self) -> bool {
        self.core.is_streaming
    }

    // --- Tool call streaming ---

    /// Create a placeholder `ToolCall` entry and record its history index.
    ///
    /// Called when `ToolUseStarted` arrives — the tool name is known but arguments
    /// are still streaming in.
    pub fn begin_tool_call(&mut self, index: usize, id: &str, name: &str) {
        let entry = ChatEntry::tool_call(id, name, "");
        let history_index = self.push_entry(entry);
        self.core
            .streaming_tool_call_indices
            .insert(index, history_index);
    }

    /// Append an incremental delta to a streaming tool call's arguments.
    ///
    /// `partial_json` is appended to the existing arguments string — it is *not*
    /// the accumulated total.
    ///
    /// # Panics
    ///
    /// Panics if no tool call entry is tracked for the given stream index.
    #[expect(
        clippy::indexing_slicing,
        reason = "index comes from push_entry which always returns a valid index"
    )]
    #[expect(
        clippy::expect_used,
        reason = "stream index is always tracked before delta arrives"
    )]
    pub fn append_tool_call_delta(&mut self, index: usize, partial_json: &str) {
        let history_index = self
            .core
            .streaming_tool_call_indices
            .get(&index)
            .copied()
            .expect("append_tool_call_delta: no entry tracked for this stream index");
        if let ChatEntryKind::ToolCall {
            ref mut arguments, ..
        } = self.core.history[history_index].kind
        {
            arguments.push_str(partial_json);
        }
    }

    /// Overwrite a tool call entry with the final complete arguments.
    ///
    /// Searches recent history for a `ToolCall` entry matching the given ID.
    /// If not found (shouldn't happen in normal flow), pushes a new entry.
    #[cfg(test)]
    pub(crate) fn finalize_tool_call(&mut self, id: &str, name: &str, arguments: &str) {
        for entry in self.core.history.iter_mut().rev() {
            if let ChatEntryKind::ToolCall {
                id: ref entry_id, ..
            } = entry.kind
                && entry_id == id
            {
                entry.kind = ChatEntryKind::ToolCall {
                    id: id.to_owned(),
                    name: name.to_owned(),
                    arguments: arguments.to_owned(),
                };
                return;
            }
        }
        // If not found (shouldn't happen), push a new entry.
        self.push_entry(ChatEntry::tool_call(id, name, arguments));
    }
}
