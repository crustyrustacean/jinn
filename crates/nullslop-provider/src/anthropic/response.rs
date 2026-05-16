//! Response parser for Anthropic SSE streaming.
//!
//! Maps Anthropic's `content_block_start/delta/stop` and `message_delta`
//! events to [`StreamEvent`] variants. Key differences from OpenAI:
//!
//! - `content_block_start` with `type: "tool_use"` → `ToolUseStart`
//! - `content_block_delta` with `type: "text_delta"` → `Text`
//! - `content_block_delta` with `type: "input_json_delta"` → `ToolUseInputDelta`
//! - `content_block_stop` → `ToolUseComplete` (drains state)
//! - `message_delta` with `stop_reason` → `Done`

use std::collections::HashMap;

use crate::StreamEvent;
use crate::stream_event::StopReason;
use crate::tool_types::ToolCall;

/// State tracked per content block index for tool use.
#[derive(Debug, Default)]
struct ToolUseState {
    id: String,
    name: String,
    json_buffer: String,
}

/// Stateful parser for Anthropic streaming responses.
#[derive(Debug, Default)]
pub struct AnthropicStreamParser {
    /// Per-index tool use state.
    tool_states: HashMap<usize, ToolUseState>,
}

impl AnthropicStreamParser {
    /// Create a new parser.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse a single SSE data payload into an optional `StreamEvent`.
    pub fn parse_data(&mut self, json: &str) -> Option<StreamEvent> {
        let response: serde_json::Value = serde_json::from_str(json).ok()?;

        let response_type = response.get("type")?.as_str()?;
        let index = response.get("index").and_then(serde_json::Value::as_u64).unwrap_or(0) as usize;

        match response_type {
            "content_block_start" => {
                let content_block = response.get("content_block")?;
                let block_type = content_block
                    .get("type")
                    .and_then(|t| t.as_str())
                    .unwrap_or("");

                if block_type == "tool_use" {
                    let id = content_block
                        .get("id")
                        .and_then(|i| i.as_str())
                        .unwrap_or("")
                        .to_owned();
                    let name = content_block
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("")
                        .to_owned();

                    self.tool_states.insert(
                        index,
                        ToolUseState {
                            id: id.clone(),
                            name: name.clone(),
                            json_buffer: String::new(),
                        },
                    );

                    Some(StreamEvent::ToolUseStart { index, id, name })
                } else {
                    None
                }
            }
            "content_block_delta" => {
                let delta = response.get("delta")?;
                let delta_type = delta.get("type").and_then(|t| t.as_str()).unwrap_or("");

                match delta_type {
                    "text_delta" => delta
                        .get("text")
                        .and_then(|t| t.as_str())
                        .map(|t| StreamEvent::Text(t.to_owned())),
                    "input_json_delta" => {
                        let partial_json = delta
                            .get("partial_json")
                            .and_then(|p| p.as_str())
                            .unwrap_or("");

                        if partial_json.is_empty() {
                            None
                        } else {
                            if let Some(state) = self.tool_states.get_mut(&index) {
                                state.json_buffer.push_str(partial_json);
                            }
                            Some(StreamEvent::ToolUseInputDelta {
                                index,
                                partial_json: partial_json.to_owned(),
                            })
                        }
                    }
                    _ => None,
                }
            }
            "content_block_stop" => {
                if let Some(state) = self.tool_states.remove(&index) {
                    // Anthropic requires tool_use.input to be a JSON object.
                    // Default to "{}" if no input deltas were received.
                    let arguments = if state.json_buffer.is_empty() {
                        "{}".to_owned()
                    } else {
                        state.json_buffer
                    };
                    Some(StreamEvent::ToolUseComplete {
                        index,
                        tool_call: ToolCall {
                            id: state.id,
                            name: state.name,
                            arguments,
                        },
                    })
                } else {
                    None
                }
            }
            "message_delta" => {
                let delta = response.get("delta")?;
                let stop_reason = delta
                    .get("stop_reason")
                    .and_then(|s| s.as_str())
                    .unwrap_or("end_turn");

                let stop = match stop_reason {
                    "tool_use" => StopReason::ToolUse,
                    "end_turn" => StopReason::EndTurn,
                    other => StopReason::Other(other.to_owned()),
                };
                Some(StreamEvent::Done { stop_reason: stop })
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_single(json: &str) -> Option<StreamEvent> {
        let mut parser = AnthropicStreamParser::new();
        parser.parse_data(json)
    }

    #[rstest::rstest]
    fn text_delta_produces_text_event() {
        let json = r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#;
        let event = parse_single(json);
        assert_eq!(event, Some(StreamEvent::Text("Hello".to_owned())));
    }

    #[rstest::rstest]
    fn tool_use_start_produces_tool_use_start() {
        let json = r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_01","name":"get_weather","input":{}}}"#;
        let event = parse_single(json);
        assert_eq!(
            event,
            Some(StreamEvent::ToolUseStart {
                index: 1,
                id: "toolu_01".to_owned(),
                name: "get_weather".to_owned(),
            })
        );
    }

    #[rstest::rstest]
    fn input_json_delta_accumulates() {
        let mut parser = AnthropicStreamParser::new();

        // Start.
        parser.parse_data(
            r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_01","name":"echo","input":{}}}"#,
        );

        // Delta.
        let event = parser.parse_data(
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"key\":"}}"#,
        );
        assert!(matches!(
            event,
            Some(StreamEvent::ToolUseInputDelta { partial_json, .. })
            if partial_json == "{\"key\":"
        ));
    }

    #[rstest::rstest]
    fn content_block_stop_produces_tool_use_complete() {
        let mut parser = AnthropicStreamParser::new();

        parser.parse_data(
            r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_01","name":"echo","input":{}}}"#,
        );
        parser.parse_data(
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"x\":1}"}}"#,
        );

        let event = parser.parse_data(r#"{"type":"content_block_stop","index":1}"#);

        match event {
            Some(StreamEvent::ToolUseComplete { tool_call, .. }) => {
                assert_eq!(tool_call.name, "echo");
                assert_eq!(tool_call.arguments, "{\"x\":1}");
            }
            other => panic!("Expected ToolUseComplete, got {other:?}"),
        }
    }

    #[rstest::rstest]
    fn empty_tool_arguments_default_to_empty_object() {
        let mut parser = AnthropicStreamParser::new();

        parser.parse_data(
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_01","name":"get_time","input":{}}}"#,
        );

        let event = parser.parse_data(r#"{"type":"content_block_stop","index":0}"#);

        match event {
            Some(StreamEvent::ToolUseComplete { tool_call, .. }) => {
                assert_eq!(tool_call.arguments, "{}");
            }
            other => panic!("Expected ToolUseComplete, got {other:?}"),
        }
    }

    #[rstest::rstest]
    fn message_delta_stop_reason_end_turn() {
        let json = r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"}}"#;
        let event = parse_single(json);
        assert_eq!(
            event,
            Some(StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
            })
        );
    }

    #[rstest::rstest]
    fn message_delta_stop_reason_tool_use() {
        let json = r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#;
        let event = parse_single(json);
        assert_eq!(
            event,
            Some(StreamEvent::Done {
                stop_reason: StopReason::ToolUse,
            })
        );
    }

    #[rstest::rstest]
    fn unknown_event_type_produces_none() {
        let json = r#"{"type":"ping"}"#;
        assert!(parse_single(json).is_none());
    }

    #[rstest::rstest]
    fn invalid_json_produces_none() {
        assert!(parse_single("not json").is_none());
    }
}
