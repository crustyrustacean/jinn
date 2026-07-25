//! `restart_mcp_server` built-in tool — lets an LLM restart a dead MCP server
//! by name (or by stripping a `mcp__<server>__<tool>` namespace) and learn
//! whether the restart succeeded before retrying the original tool call.
//!
//! See the plan at `.plans/mcp-restart-tool/plan.md`.

use std::time::Duration;

use futures::{FutureExt, future::BoxFuture};
use kameo::actor::{ActorRef, Spawn};
use kameo::prelude::{Context, Message};

use crate::feat::mcp_actor::protocol::{McpConnectionStatus, McpServerStatus};
use crate::feat::mcp_coordinator_actor::protocol::RestartMcpServer;
use crate::feat::tools_actor::tool_types::{ToolCall, ToolContext, ToolDefinition, ToolResult};
use crate::protocol::SessionId;

use super::BoxedToolFuture;

/// How long `execute()` waits for the restarted server to reach `Running` or
/// `Dead` before reporting "still starting". Generous: slow HTTP/Python servers
/// can legitimately boot for 45s+. [`execute_with_timeout`] lets tests inject
/// a smaller value.
pub const RESTART_TIMEOUT: Duration = Duration::from_mins(1);

/// Returns the tool definition for the `restart_mcp_server` built-in tool.
pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "restart_mcp_server".to_owned(),
        description: "Restart a dead or broken MCP server so its tools work again. \
            Call this when an `mcp__<server>__*` tool call fails (the server has likely died). \
            Pass the server's name (e.g. `excalimate`) — a full `mcp__<server>__<tool>` tool name \
            is also accepted and silently resolved. This waits until the server has finished \
            restarting, then returns whether it is back online. After a successful restart, retry \
            the original tool call. \
            **If this tool reports failure, STOP and wait for user instruction — do not retry.**"
            .to_owned(),
        prompt_snippet: None,
        prompt_guidelines: vec![],
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "server": {
                    "type": "string",
                    "description": "The MCP server to restart, by name (e.g. `excalimate`), \
                        or the failing `mcp__<server>__<tool>` tool name (the namespace is stripped)."
                }
            },
            "required": ["server"]
        }),
        server_tool_type: None,
    }
}

/// Executes the `restart_mcp_server` built-in tool with the default 60s timeout.
pub fn execute(call: ToolCall, ctx: ToolContext) -> BoxedToolFuture {
    execute_with_timeout(call, ctx, RESTART_TIMEOUT)
}

/// Same as [`execute`] but with an injectable timeout (for tests).
pub fn execute_with_timeout(
    call: ToolCall,
    ctx: ToolContext,
    timeout: Duration,
) -> BoxedToolFuture {
    let tool_call_id = call.id;
    let tool_name = call.name;
    let args_str = call.arguments;

    // Parse args + resolve the server up front; surface clear failures.
    let Some(state) = ctx.state else {
        return failure_future(&tool_call_id, &tool_name, "no application state available");
    };
    let Some(session_id) = ctx.session_id else {
        return failure_future(&tool_call_id, &tool_name, "no active session");
    };
    let Some(bus) = ctx.bus else {
        return failure_future(&tool_call_id, &tool_name, "no bus available");
    };

    let server = match parse_args(&args_str, &state) {
        Ok(server) => server,
        Err(msg) => return failure_future(&tool_call_id, &tool_name, &msg),
    };
    async move {
        // Subscribe BEFORE publishing restart so we cannot miss the new actor's
        // status transitions (see `StatusCollector` doc on race-ordering).
        let mut statuses =
            match StatusCollector::start(&bus, session_id.clone(), server.clone()).await {
                Ok(s) => s,
                Err(e) => {
                    return failure_result(
                        &tool_call_id,
                        &tool_name,
                        &format!("failed to subscribe to MCP status: {e}"),
                    );
                }
            };

        // Kick off the restart (fire-and-forget command; we observe via the bus).
        bus.publish(RestartMcpServer {
            session_id,
            server: server.clone(),
        })
        .await;

        // Wait for the new server's terminal status (Running/Dead), or timeout.
        // Per Gotcha #3: the kill+spawn sequence emits (old)Dead -> (new)Starting
        // -> (new)terminal. We ignore everything until the first Starting-after-
        // publish, then take the terminal status after it. This avoids treating
        // the old actor's teardown Dead as the new server's failure.
        match wait_for_terminal(&mut statuses, timeout).await {
            Outcome::Running => ToolResult {
                tool_call_id,
                name: tool_name,
                content: format!(
                    "MCP server `{server}` restarted successfully and is back online. \
                     MCP tools are available again — retry your original tool call."
                ),
                success: true,
                full_content: None,
                truncation: None,
                pin_position: None,
            },
            Outcome::Dead => ToolResult {
                tool_call_id,
                name: tool_name,
                content: format!(
                    "MCP server `{server}` failed to restart — it is dead. \
                     **STOP and wait for user instruction.** Do not retry; the user \
                     should check the server config in the inspector (`<leader>sM`)."
                ),
                success: false,
                full_content: None,
                truncation: None,
                pin_position: None,
            },
            Outcome::StillStarting => ToolResult {
                tool_call_id,
                name: tool_name,
                content: format!(
                    "MCP server `{server}` is still starting after {timeout:?}. \
                     It is likely a slow-to-boot server (HTTP/Python). Retry your \
                     original tool call shortly."
                ),
                success: true,
                full_content: None,
                truncation: None,
                pin_position: None,
            },
            Outcome::Lost => failure_result(
                &tool_call_id,
                &tool_name,
                "MCP status subscriber was lost mid-restart; retry the call.",
            ),
        }
    }
    .boxed()
}

/// The observable outcome of waiting for a restart to resolve.
enum Outcome {
    Running,
    Dead,
    StillStarting,
    Lost,
}

/// Wait for the new server's terminal status, applying the kill+spawn ordering
/// guard (Gotcha #3): ignore statuses until the first `Starting` *after* our
/// restart publish, then take the terminal status after it.
///
/// Returns `Outcome::Lost` if the stream ends (collector dropped) before any
/// terminal status, and `Outcome::StillStarting` if the timeout fires.
async fn wait_for_terminal(statuses: &mut StatusStream, timeout: Duration) -> Outcome {
    // Phase 1: ignore everything until we see the new actor's Starting. The
    // old actor's teardown Dead may arrive first; we must not treat it as the
    // new server's failure.
    loop {
        match tokio::time::timeout(timeout, statuses.next()).await {
            Ok(Some(status)) if status.status == McpConnectionStatus::Starting => break,
            Ok(Some(_)) => {} // ignore pre-Starting (old actor's Dead)
            Ok(None) => return Outcome::Lost,
            Err(_) => return Outcome::StillStarting,
        }
    }

    // Phase 2: wait for the terminal status (Running/Dead). Ignore further
    // Starting states (a server shouldn't emit them post-start, but be lenient).
    loop {
        match tokio::time::timeout(timeout, statuses.next()).await {
            Ok(Some(status)) => match status.status {
                McpConnectionStatus::Running => return Outcome::Running,
                McpConnectionStatus::Dead => return Outcome::Dead,
                McpConnectionStatus::Starting => {}
            },
            Ok(None) => return Outcome::Lost,
            Err(_) => return Outcome::StillStarting,
        }
    }
}

// ---------------------------------------------------------------------------
// Server-name resolution + arg parsing
// ---------------------------------------------------------------------------

/// Parses the tool's `server` argument and resolves it to a configured server.
///
/// Accepts either a bare server name (`excalimate`) or a full
/// `mcp__<server>__<tool>` tool name (the namespace is stripped silently).
/// Returns `Err(message)` on missing/invalid args or an unknown server.
fn parse_args(args_str: &str, state: &crate::common::state::State) -> Result<String, String> {
    let value: serde_json::Value = if args_str.trim().is_empty() {
        return Err("missing `server` argument".to_owned());
    } else {
        serde_json::from_str(args_str).map_err(|e| format!("invalid arguments: {e}"))?
    };
    let Some(input) = value.get("server").and_then(serde_json::Value::as_str) else {
        return Err("`server` argument must be a string".to_owned());
    };
    let configured = configured_server_names(state);
    resolve_server(input, &configured).ok_or_else(|| format!("unknown MCP server `{input}`"))
}

/// Reads the configured server names from application state.
fn configured_server_names(state: &crate::common::state::State) -> Vec<String> {
    state
        .read()
        .frontend
        .preferences
        .mcp_servers
        .iter()
        .map(|s| s.name.clone())
        .collect()
}

/// Resolves a raw input to a configured server name.
///
/// - Exact match against `configured` → that name.
/// - `mcp__<X>__<rest>` form where `X` is configured → `X`.
/// - Otherwise `None`.
fn resolve_server(input: &str, configured: &[String]) -> Option<String> {
    // Exact match.
    if let Some(found) = configured.iter().find(|c| c.as_str() == input) {
        return Some(found.clone());
    }
    // Namespace-stripped form: mcp__<server>__<tool>.
    if let Some(rest) = input.strip_prefix("mcp__")
        && let Some((server, _tool)) = rest.split_once("__")
        && configured.iter().any(|c| c == server)
    {
        return Some(server.to_owned());
    }
    None
}

// ---------------------------------------------------------------------------
// Result helpers
// ---------------------------------------------------------------------------

fn failure_future(call_id: &str, name: &str, message: &str) -> BoxedToolFuture {
    futures::future::ready(failure_result(call_id, name, message)).boxed()
}

fn failure_result(call_id: &str, name: &str, message: &str) -> ToolResult {
    ToolResult {
        tool_call_id: call_id.to_owned(),
        name: name.to_owned(),
        content: message.to_owned(),
        success: false,
        full_content: None,
        truncation: None,
        pin_position: None,
    }
}

// ---------------------------------------------------------------------------
// StatusCollector — transient bus subscriber
// ---------------------------------------------------------------------------

/// A transient kameo actor that subscribes to `McpServerStatus` on the bus and
/// forwards every status for the target `(session_id, server)` into a channel
/// the tool's future reads.
///
/// Why a dedicated actor: `BusService` subscriptions are recipient-based
/// (`kameo::actor::Recipient`), and a builtin tool's `execute()` is a free
/// function with no actor context. We spawn this actor, subscribe it, then read
/// the channel until the terminal status arrives — at which point the tool
/// drops the stream, the collector goes idle, and it stops itself on drop.
///
/// Race-ordering: subscribe **before** publishing `RestartMcpServer`. The
/// kill+spawn sequence emits `(old)Dead -> (new)Starting -> (new)terminal`; the
/// tool ignores pre-Starting events so the old actor's teardown Dead is never
/// mistaken for the new server's failure.
struct StatusCollector {
    tx: kanal::Sender<McpServerStatus>,
    session_id: SessionId,
    server: String,
}

impl StatusCollector {
    /// Subscribe to `McpServerStatus` on the bus and return a stream handle.
    fn start(
        bus: &crate::common::services::bus_service::BusService,
        session_id: SessionId,
        server: String,
    ) -> BoxFuture<'static, Result<StatusStream, String>> {
        let bus = bus.clone();
        async move {
            let (tx, rx) = kanal::unbounded::<McpServerStatus>();
            let actor = Spawn::spawn(StatusCollectorArgs {
                tx,
                session_id,
                server,
            });
            actor.wait_for_startup().await;
            bus.subscribe::<McpServerStatus, StatusCollector>(&actor)
                .await;
            Ok(StatusStream {
                rx: rx.to_async(),
                _actor: actor,
            })
        }
        .boxed()
    }
}

/// Spawn args for [`StatusCollector`] (carries the channel + filter keys).
struct StatusCollectorArgs {
    tx: kanal::Sender<McpServerStatus>,
    session_id: SessionId,
    server: String,
}

impl kameo::Actor for StatusCollector {
    type Args = StatusCollectorArgs;
    type Error = kameo::error::Infallible;

    async fn on_start(args: Self::Args, _actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        Ok(Self {
            tx: args.tx,
            session_id: args.session_id,
            server: args.server,
        })
    }
}

impl Message<McpServerStatus> for StatusCollector {
    type Reply = ();

    async fn handle(&mut self, msg: McpServerStatus, _ctx: &mut Context<Self, Self::Reply>) {
        // Only forward statuses for the target (session × server).
        if msg.session_id == self.session_id && msg.server == self.server {
            let _ = self.tx.send(msg);
        }
    }
}

/// Handle returned to the tool; yields the collected statuses.
struct StatusStream {
    rx: kanal::AsyncReceiver<McpServerStatus>,
    /// Kept alive so the actor isn't dropped while the tool is still waiting.
    _actor: ActorRef<StatusCollector>,
}

impl StatusStream {
    /// Yields the next matching status, or `None` if the collector stopped.
    async fn next(&mut self) -> Option<McpServerStatus> {
        self.rx.recv().await.ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::state::State;

    fn configured() -> Vec<String> {
        vec!["excalimate".to_owned(), "context7".to_owned()]
    }

    #[test]
    fn resolve_server_exact_match_returns_name() {
        // Given a configured server list and a bare server name.
        // When resolving.
        let resolved = resolve_server("excalimate", &configured());

        // Then the bare name is returned.
        assert_eq!(resolved.as_deref(), Some("excalimate"));
    }

    #[test]
    fn resolve_server_strips_mcp_namespace() {
        // Given a configured server list and a full mcp__ namespaced tool name.
        // When resolving.
        let resolved = resolve_server("mcp__excalimate__create_scene", &configured());

        // Then the server name is extracted from the namespace.
        assert_eq!(resolved.as_deref(), Some("excalimate"));
    }

    #[test]
    fn resolve_server_returns_none_for_unknown_server() {
        // Given a name that is not configured.
        // When resolving.
        let resolved = resolve_server("nonexistent", &configured());

        // Then it returns None.
        assert!(resolved.is_none());
    }

    #[test]
    fn resolve_server_returns_none_for_unknown_namespace() {
        // Given a namespaced tool whose server is not configured.
        // When resolving.
        let resolved = resolve_server("mcp__unknown__tool", &configured());

        // Then it returns None.
        assert!(resolved.is_none());
    }

    #[test]
    fn parse_args_rejects_missing_server_argument() {
        // Given a state with configured servers but no server arg.
        let state = State::new(crate::common::app_state::AppState::default());

        // When parsing empty args.
        let result = parse_args("", &state);

        // Then it errors with a clear message.
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("server"),
            "error should mention server"
        );
    }

    #[test]
    fn parse_args_rejects_non_string_server() {
        // Given a state and a numeric server arg.
        let state = State::new(crate::common::app_state::AppState::default());

        // When parsing args with a non-string server.
        let result = parse_args(r#"{"server": 42}"#, &state);

        // Then it errors.
        assert!(result.is_err());
    }

    #[test]
    fn parse_args_rejects_unknown_server() {
        // Given a state and an unconfigured server name.
        let state = State::new(crate::common::app_state::AppState::default());

        // When parsing args with an unknown server.
        let result = parse_args(r#"{"server": "ghost"}"#, &state);

        // Then it errors with the server name in the message.
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("ghost"));
    }

    #[test]
    fn definition_has_teaching_description_with_stop_instruction() {
        // Given the tool definition.
        // When reading its description.
        let def = definition();

        // Then it teaches when to call and includes the STOP instruction.
        assert_eq!(def.name, "restart_mcp_server");
        assert!(
            def.description.contains("mcp__"),
            "should teach the model to call on mcp tool failure"
        );
        assert!(
            def.description.contains("STOP"),
            "should include the STOP-and-wait instruction"
        );
    }
}
