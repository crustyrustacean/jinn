//! Response parser for OpenAI-compatible SSE streaming.
//!
//! Maps parsed SSE JSON payloads to [`StreamEvent`] variants. Handles
//! tool call state tracking across multiple delta chunks and deduplication
//! of `Done` events (OpenRouter sends `finish_reason` then `[DONE]`).

use std::collections::HashMap;

use crate::StreamEvent;
use crate::stream_event::StopReason;
use crate::tool_types::ToolCall;

/// State tracked per tool call index during streaming.
#[derive(Debug, Default)]
struct ToolCallState {
    /// Tool call ID (set on first chunk).
    id: String,
    /// Function name (set on first chunk).
    name: String,
    /// Accumulated JSON arguments.
    arguments_buffer: String,
    /// Whether we've emitted a `ToolUseStart` for this tool call.
    started: bool,
}

/// Stateful parser that tracks tool call accumulation across SSE chunks.
#[derive(Debug, Default)]
pub struct StreamResponseParser {
    /// Per-index tool call state.
    tool_states: HashMap<usize, ToolCallState>,
    /// Whether a Done event has already been emitted (prevents duplicates).
    done_emitted: bool,
}

impl StreamResponseParser {
    /// Create a new parser.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse a single SSE data payload into zero or more `StreamEvent`s.
    ///
    /// Handles:
    /// - `delta.content` → `StreamEvent::Text`
    /// - `delta.reasoning_content` → `StreamEvent::Reasoning`
    /// - `delta.tool_calls[]` → `ToolUseStart`/`ToolUseInputDelta` (state tracked)
    /// - `finish_reason` → `ToolUseComplete` + `Done` (drains pending tool calls)
    #[allow(clippy::collapsible_if, clippy::manual_let_else)]
    pub fn parse_data(&mut self, json: &str) -> Vec<StreamEvent> {
        let mut results = Vec::new();

        let chunk: serde_json::Value = match serde_json::from_str(json) {
            Ok(v) => v,
            Err(_) => return results,
        };

        let choices = match chunk.get("choices").and_then(|c| c.as_array()) {
            Some(c) => c,
            None => return results,
        };

        for choice in choices {
            let delta = match choice.get("delta") {
                Some(d) => d,
                None => continue,
            };

            // Text content delta.
            if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
                if !content.is_empty() {
                    results.push(StreamEvent::Text(content.to_owned()));
                }
            }

            // Reasoning/thinking content delta.
            if let Some(reasoning) = delta.get("reasoning_content").and_then(|c| c.as_str()) {
                if !reasoning.is_empty() {
                    results.push(StreamEvent::Reasoning(reasoning.to_owned()));
                }
            }

            // Tool call deltas.
            if let Some(tool_calls) = delta.get("tool_calls").and_then(|t| t.as_array()) {
                for tc in tool_calls {
                    let index = tc
                        .get("index")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0) as usize;

                    let state = self.tool_states.entry(index).or_default();

                    // First chunk: id and name.
                    if let Some(id) = tc.get("id").and_then(|i| i.as_str()) {
                        id.clone_into(&mut state.id);
                    }

                    let function = tc.get("function");

                    if let Some(name) = function
                        .and_then(|f| f.get("name"))
                        .and_then(|n| n.as_str())
                    {
                        name.clone_into(&mut state.name);

                        if !state.started {
                            state.started = true;
                            results.push(StreamEvent::ToolUseStart {
                                index,
                                id: state.id.clone(),
                                name: state.name.clone(),
                            });
                        }
                    }

                    // Arguments delta.
                    let arguments = function
                        .and_then(|f| f.get("arguments"))
                        .and_then(|a| a.as_str())
                        .unwrap_or("");

                    if !arguments.is_empty() {
                        state.arguments_buffer.push_str(arguments);
                        results.push(StreamEvent::ToolUseInputDelta {
                            index,
                            partial_json: arguments.to_owned(),
                        });
                    }
                }
            }

            // Finish reason.
            if let Some(finish_reason) = choice.get("finish_reason").and_then(|f| f.as_str()) {
                if !finish_reason.is_empty() && !self.done_emitted {
                    // Drain all pending tool calls.
                    let mut pending_indices: Vec<usize> =
                        self.tool_states.keys().copied().collect();
                    pending_indices.sort_unstable();

                    for idx in pending_indices {
                        if let Some(state) = self.tool_states.remove(&idx) {
                            if state.started {
                                results.push(StreamEvent::ToolUseComplete {
                                    index: idx,
                                    tool_call: ToolCall {
                                        id: state.id,
                                        name: state.name,
                                        arguments: state.arguments_buffer,
                                    },
                                });
                            }
                        }
                    }

                    let stop_reason = match finish_reason {
                        "tool_calls" => StopReason::ToolUse,
                        "stop" => StopReason::EndTurn,
                        other => StopReason::Other(other.to_string()),
                    };

                    results.push(StreamEvent::Done { stop_reason });
                    self.done_emitted = true;
                }
            }
        }

        results
    }

    /// Handle the `[DONE]` sentinel from SSE.
    ///
    /// If `finish_reason` already emitted a `Done`, this is a no-op
    /// (prevents the OpenRouter double-Done bug).
    /// Otherwise, drains any pending tool calls and emits `Done(EndTurn)`.
    #[allow(clippy::collapsible_if)]
    pub fn handle_done(&mut self) -> Vec<StreamEvent> {
        if self.done_emitted {
            return vec![];
        }

        let mut results = Vec::new();

        // Drain any remaining tool calls.
        let mut pending_indices: Vec<usize> = self.tool_states.keys().copied().collect();
        pending_indices.sort_unstable();

        for idx in pending_indices {
            if let Some(state) = self.tool_states.remove(&idx) {
                if state.started {
                    results.push(StreamEvent::ToolUseComplete {
                        index: idx,
                        tool_call: ToolCall {
                            id: state.id,
                            name: state.name,
                            arguments: state.arguments_buffer,
                        },
                    });
                }
            }
        }

        let has_remaining_tools = !results.is_empty();
        let stop_reason = if has_remaining_tools {
            StopReason::ToolUse
        } else {
            StopReason::EndTurn
        };

        results.push(StreamEvent::Done { stop_reason });
        self.done_emitted = true;
        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_single(json: &str) -> Vec<StreamEvent> {
        let mut parser = StreamResponseParser::new();
        parser.parse_data(json)
    }

    #[rstest::rstest]
    fn text_delta_produces_text_event() {
        let json = r#"{"id":"x","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}"#;
        let events = parse_single(json);

        assert_eq!(events.len(), 1);
        assert_eq!(events[0], StreamEvent::Text("Hello".to_owned()));
    }

    #[rstest::rstest]
    fn empty_content_produces_no_event() {
        let json =
            r#"{"id":"x","choices":[{"index":0,"delta":{"content":""},"finish_reason":null}]}"#;
        let events = parse_single(json);
        assert!(events.is_empty());
    }

    #[rstest::rstest]
    fn reasoning_delta_produces_reasoning_event() {
        let json = r#"{"id":"x","choices":[{"index":0,"delta":{"reasoning_content":"thinking..."},"finish_reason":null}]}"#;
        let events = parse_single(json);

        assert_eq!(events.len(), 1);
        assert_eq!(events[0], StreamEvent::Reasoning("thinking...".to_owned()));
    }

    #[rstest::rstest]
    fn tool_call_start_produces_tool_use_start() {
        let json = r#"{"id":"x","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"get_weather","arguments":""}}]},"finish_reason":null}]}"#;
        let events = parse_single(json);

        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0],
            StreamEvent::ToolUseStart {
                index: 0,
                id: "call_1".to_owned(),
                name: "get_weather".to_owned(),
            }
        );
    }

    #[rstest::rstest]
    fn tool_call_arguments_delta_accumulates() {
        let mut parser = StreamResponseParser::new();

        // Start.
        let start_json = r#"{"id":"x","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"echo","arguments":""}}]},"finish_reason":null}]}"#;
        let events1 = parser.parse_data(start_json);
        assert!(matches!(&events1[0], StreamEvent::ToolUseStart { name, .. } if name == "echo"));

        // Arguments delta.
        let delta_json = r#"{"id":"x","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"key\":"}}]},"finish_reason":null}]}"#;
        let events2 = parser.parse_data(delta_json);
        assert_eq!(events2.len(), 1);
        assert!(matches!(
            &events2[0],
            StreamEvent::ToolUseInputDelta { partial_json, .. }
            if partial_json == "{\"key\":"
        ));
    }

    #[rstest::rstest]
    fn finish_reason_stop_produces_done_end_turn() {
        let json = r#"{"id":"x","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#;
        let events = parse_single(json);

        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0],
            StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
            }
        );
    }

    #[rstest::rstest]
    fn finish_reason_tool_calls_produces_complete_and_done() {
        let mut parser = StreamResponseParser::new();

        // Set up a tool call.
        let start_json = r#"{"id":"x","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"echo","arguments":""}}]},"finish_reason":null}]}"#;
        parser.parse_data(start_json);

        // Simulate some arguments.
        let args_json = r#"{"id":"x","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"x\":1}"}}]},"finish_reason":null}]}"#;
        parser.parse_data(args_json);

        // Finish.
        let finish_json =
            r#"{"id":"x","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#;
        let events = parser.parse_data(finish_json);

        assert_eq!(events.len(), 2);
        assert!(
            matches!(&events[0], StreamEvent::ToolUseComplete { index: 0, tool_call } if tool_call.name == "echo" && tool_call.arguments == "{\"x\":1}")
        );
        assert!(matches!(
            &events[1],
            StreamEvent::Done {
                stop_reason: StopReason::ToolUse
            }
        ));
    }

    #[rstest::rstest]
    fn done_sentinel_after_finish_is_noop() {
        let mut parser = StreamResponseParser::new();

        // Finish with stop.
        let json = r#"{"id":"x","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#;
        parser.parse_data(json);

        // [DONE] sentinel.
        let events = parser.handle_done();
        assert!(events.is_empty());
    }

    #[rstest::rstest]
    fn done_sentinel_without_finish_produces_done() {
        let mut parser = StreamResponseParser::new();
        let events = parser.handle_done();

        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0],
            StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
            }
        );
    }

    #[rstest::rstest]
    fn parallel_tool_calls_start() {
        let json = r#"{"id":"x","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"f1","arguments":""}},{"index":1,"id":"c2","function":{"name":"f2","arguments":""}}]},"finish_reason":null}]}"#;
        let events = parse_single(json);

        assert_eq!(events.len(), 2);
        assert!(matches!(&events[0], StreamEvent::ToolUseStart { name, .. } if name == "f1"));
        assert!(matches!(&events[1], StreamEvent::ToolUseStart { name, .. } if name == "f2"));
    }

    #[rstest::rstest]
    fn invalid_json_produces_no_events() {
        let events = parse_single("not json");
        assert!(events.is_empty());
    }

    #[rstest::rstest]
    fn no_choices_produces_no_events() {
        let events = parse_single(r#"{"id":"x"}"#);
        assert!(events.is_empty());
    }

    #[rstest::rstest]
    fn done_sentinel_with_pending_tool_calls() {
        let mut parser = StreamResponseParser::new();

        // Start a tool call.
        let start_json = r#"{"id":"x","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"echo","arguments":""}}]},"finish_reason":null}]}"#;
        parser.parse_data(start_json);

        // [DONE] without finish_reason.
        let events = parser.handle_done();

        assert_eq!(events.len(), 2);
        assert!(matches!(&events[0], StreamEvent::ToolUseComplete { .. }));
        assert!(matches!(
            &events[1],
            StreamEvent::Done {
                stop_reason: StopReason::ToolUse,
            }
        ));
    }

    #[rstest::rstest]
    fn full_tool_call_sequence() {
        let mut parser = StreamResponseParser::new();

        // 1. Start.
        let e1 = parser.parse_data(
            r#"{"id":"x","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"get_weather","arguments":""}}]},"finish_reason":null}]}"#,
        );
        assert!(matches!(&e1[0], StreamEvent::ToolUseStart { name, .. } if name == "get_weather"));

        // 2. Arguments delta.
        let e2 = parser.parse_data(
            r#"{"id":"x","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"city\":"}}]},"finish_reason":null}]}"#,
        );
        assert!(
            matches!(&e2[0], StreamEvent::ToolUseInputDelta { partial_json, .. } if partial_json == "{\"city\":")
        );

        // 3. More arguments.
        let e3 = parser.parse_data(
            r#"{"id":"x","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"Paris\"}"}}]},"finish_reason":null}]}"#,
        );
        assert!(
            matches!(&e3[0], StreamEvent::ToolUseInputDelta { partial_json, .. } if partial_json == "\"Paris\"}")
        );

        // 4. Finish.
        let e4 = parser.parse_data(
            r#"{"id":"x","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#,
        );
        assert_eq!(e4.len(), 2);
        let tc = match &e4[0] {
            StreamEvent::ToolUseComplete { tool_call, .. } => tool_call.clone(),
            _ => panic!("expected ToolUseComplete"),
        };
        assert_eq!(tc.name, "get_weather");
        assert_eq!(tc.arguments, "{\"city\":\"Paris\"}");
    }
}
