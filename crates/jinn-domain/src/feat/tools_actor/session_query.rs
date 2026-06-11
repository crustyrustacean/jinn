//! `session_query` built-in tool — lets an LLM query another session's chat history.
//!
//! Used by judge plugins to inspect origin session conversations. Supports two
//! actions: `get_recent` (last N entries) and `search` (text matching). Always
//! registered as a built-in tool alongside `bash`, `read`, etc.

use crate::common::state::State;
use crate::feat::session::chat_entry::ChatEntryKind;
use crate::feat::tools_actor::tool_types::{ToolCall, ToolContext, ToolDefinition, ToolResult};
use crate::protocol::SessionId;

use super::BoxedToolFuture;

/// Returns the tool definition for the `session_query` built-in tool.
pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "session_query".to_owned(),
        description: "Query another session's chat history. Use this to inspect conversations in other sessions.".to_owned(),
        prompt_snippet: None,
        prompt_guidelines: vec![],
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "session_id": {
                    "type": "string",
                    "description": "The ID of the session to query."
                },
                "action": {
                    "type": "string",
                    "enum": ["get_recent", "search"],
                    "description": "Which query to run: 'get_recent' returns the last N entries, 'search' finds entries matching text."
                },
                "count": {
                    "type": "integer",
                    "description": "For 'get_recent': number of recent entries to return. Defaults to 10."
                },
                "query": {
                    "type": "string",
                    "description": "For 'search': text to search for in entries."
                }
            },
            "required": ["session_id", "action"]
        }),
        server_tool_type: None,
    }
}

/// Executes the `session_query` built-in tool.
///
/// # Errors
///
/// Returns a `ToolResult` with `success = false` if the session is not found
/// or the arguments are invalid.
#[expect(
    clippy::needless_pass_by_value,
    reason = "all tool execute functions take ctx by value"
)]
pub fn execute(call: ToolCall, ctx: ToolContext) -> BoxedToolFuture {
    let args_str = call.arguments.clone();
    let state = ctx.state.clone();
    let tool_call_id = call.id;
    let tool_name = call.name;

    Box::pin(async move {
        let args: serde_json::Value = match serde_json::from_str(&args_str) {
            Ok(v) => v,
            Err(e) => {
                return ToolResult {
                    tool_call_id,
                    name: tool_name,
                    content: format!("Error: invalid JSON arguments: {e}"),
                    success: false,
                    full_content: None,
                    truncation: None,
                    pin_position: None,
                };
            }
        };
        let result = execute_query(&args, state.as_ref());
        ToolResult {
            tool_call_id,
            name: tool_name,
            content: result,
            success: true,
            full_content: None,
            truncation: None,
            pin_position: None,
        }
    })
}

/// Run the query and return formatted text.
fn execute_query(args: &serde_json::Value, state: Option<&State>) -> String {
    let Some(state) = state else {
        return "Error: no state available".to_owned();
    };

    let session_id = match args.get("session_id").and_then(|v| v.as_str()) {
        Some(id) => SessionId::from(id.to_owned()),
        None => return "Error: session_id is required".to_owned(),
    };

    let Some(action) = args.get("action").and_then(serde_json::Value::as_str) else {
        return "Error: action is required".to_owned();
    };

    let guard = state.read();
    let Some(session) = guard.session.get(&session_id) else {
        return format!("Error: session {session_id} not found");
    };

    match action {
        "get_recent" => {
            let count = args
                .get("count")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(10) as usize;
            let history = session.history();
            let start = history.len().saturating_sub(count);
            let entries = history.get(start..).unwrap_or_default();
            format_entries(entries)
        }
        "search" => {
            let query = match args.get("query").and_then(|v| v.as_str()) {
                Some(q) => q.to_lowercase(),
                None => return "Error: query is required for search action".to_owned(),
            };
            let history = session.history();
            let matching: Vec<crate::feat::session::chat_entry::ChatEntry> = history
                .iter()
                .filter(|e| e.text().to_lowercase().contains(&query))
                .cloned()
                .collect();
            if matching.is_empty() {
                "No matching entries found.".to_owned()
            } else {
                format_entries(&matching)
            }
        }
        _ => format!("Error: unknown action '{action}'. Use 'get_recent' or 'search'."),
    }
}

/// Format a slice of chat entries as readable text.
fn format_entries(entries: &[crate::feat::session::chat_entry::ChatEntry]) -> String {
    use std::fmt::Write;

    let mut output = String::new();
    for entry in entries {
        let kind = entry.kind_str();
        let text = entry.text();

        // Skip transient and thinking entries — not relevant for queries.
        if matches!(
            entry.kind,
            ChatEntryKind::Transient(_) | ChatEntryKind::Thinking(_)
        ) {
            continue;
        }

        let _ = writeln!(output, "[{kind}] {text}");
    }

    if output.is_empty() {
        "No entries to display.".to_owned()
    } else {
        output
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]

    use super::*;
    use crate::common::state::State;
    use crate::feat::session::chat_entry::{ChatEntry, ChatEntryKind};
    use crate::feat::session::chat_history::ChatHistory;
    use crate::feat::session::chat_session::ChatSessionState;
    use serde_json::json;

    fn make_entry(kind: ChatEntryKind) -> ChatEntry {
        ChatEntry {
            id: crate::feat::session::chat_entry::ChatEntryId::new(),
            timing: crate::feat::session::entry_timing::EntryTiming::instant_now(),
            kind,
            pin_position: None,
            context_override: crate::feat::session::chat_entry::ContextOverride::Default,
            context_history: vec![],
        }
    }

    fn make_state_with_history(entries: Vec<ChatEntry>) -> (State, SessionId) {
        let state = State::new(crate::common::app_state::AppState::default());
        let session_id = SessionId::new();
        let mut session = ChatSessionState::default();
        session.core.session_id = session_id.clone();
        session.core.history = ChatHistory::from_vec(entries);
        state.write().session.insert(session);
        (state, session_id)
    }

    #[test]
    fn get_recent_returns_last_n_entries() {
        // Given a session with 5 entries.
        let entries = vec![
            make_entry(ChatEntryKind::System("msg1".to_owned())),
            make_entry(ChatEntryKind::System("msg2".to_owned())),
            make_entry(ChatEntryKind::System("msg3".to_owned())),
            make_entry(ChatEntryKind::System("msg4".to_owned())),
            make_entry(ChatEntryKind::System("msg5".to_owned())),
        ];
        let (state, session_id) = make_state_with_history(entries);

        // When querying get_recent with count=3.
        let args = json!({
            "session_id": session_id.to_string(),
            "action": "get_recent",
            "count": 3
        });
        let result = execute_query(&args, Some(&state));

        // Then only the last 3 entries are returned.
        assert!(result.contains("msg3"));
        assert!(result.contains("msg4"));
        assert!(result.contains("msg5"));
        assert!(!result.contains("msg1"));
        assert!(!result.contains("msg2"));
    }

    #[test]
    fn search_returns_matching_entries() {
        // Given a session with mixed entries.
        let entries = vec![
            make_entry(ChatEntryKind::User {
                display: "Hello world".to_owned(),
                expanded: "Hello world".to_owned(),
            }),
            make_entry(ChatEntryKind::Assistant("Goodbye moon".to_owned())),
            make_entry(ChatEntryKind::User {
                display: "Hello again".to_owned(),
                expanded: "Hello again".to_owned(),
            }),
        ];
        let (state, session_id) = make_state_with_history(entries);

        // When searching for "hello".
        let args = json!({
            "session_id": session_id.to_string(),
            "action": "search",
            "query": "hello"
        });
        let result = execute_query(&args, Some(&state));

        // Then only matching entries are returned.
        assert!(result.contains("Hello world"));
        assert!(result.contains("Hello again"));
        assert!(!result.contains("Goodbye moon"));
    }

    #[test]
    fn invalid_session_id_returns_error() {
        // Given a query for a nonexistent session.
        let args = json!({
            "session_id": "nonexistent",
            "action": "get_recent",
            "count": 5
        });

        // When executing the query.
        let result = execute_query(&args, None);

        // Then an error is returned.
        assert!(result.contains("Error"));
    }

    #[test]
    fn empty_history_returns_no_entries() {
        // Given a session with no entries.
        let (state, session_id) = make_state_with_history(vec![]);

        // When querying get_recent.
        let args = json!({
            "session_id": session_id.to_string(),
            "action": "get_recent",
            "count": 10
        });
        let result = execute_query(&args, Some(&state));

        // Then no entries are displayed.
        assert_eq!(result, "No entries to display.");
    }

    #[test]
    fn search_with_no_matches_returns_not_found() {
        // Given a session with one entry.
        let entries = vec![make_entry(ChatEntryKind::Assistant("Hello".to_owned()))];
        let (state, session_id) = make_state_with_history(entries);

        // When searching for something that doesn't match.
        let args = json!({
            "session_id": session_id.to_string(),
            "action": "search",
            "query": "xyzzy"
        });
        let result = execute_query(&args, Some(&state));

        // Then no matching entries found.
        assert_eq!(result, "No matching entries found.");
    }

    #[test]
    fn transient_and_thinking_entries_are_excluded() {
        // Given a session with transient and thinking entries mixed in.
        let entries = vec![
            make_entry(ChatEntryKind::System("visible".to_owned())),
            make_entry(ChatEntryKind::Transient("hidden transient".to_owned())),
            make_entry(ChatEntryKind::Thinking("hidden thinking".to_owned())),
        ];
        let (state, session_id) = make_state_with_history(entries);

        // When querying get_recent.
        let args = json!({
            "session_id": session_id.to_string(),
            "action": "get_recent",
            "count": 10
        });
        let result = execute_query(&args, Some(&state));

        // Then only the system entry appears.
        assert!(result.contains("visible"));
        assert!(!result.contains("hidden"));
    }
}
