// Copyright (C) 2026 Jayson Lennon
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as
// published by the Free Software Foundation, either version 3 of the
// License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! `task` built-in tool — delegate a sub-task to a fresh subagent session.
//!
//! Spawns a regular session linked to the caller (empty history, inheriting
//! the parent's model, CWD, persona, tools, skills, and MCP servers), enqueues
//! the given prompt into it, and blocks until the child reaches
//! [`PhaseKind::Idle`](crate::feat::session::phase_machine::PhaseKind). The
//! child's last chat entry becomes the tool result. Subagents are just
//! sessions: they appear in the sidebar, can be steered, and persist.
//!
//! Ordering guarantees: the completion listener is spawned and subscribed
//! before `SessionCreated` is published (other actors react to that event),
//! and the in-flight spawn is registered before the wait begins (so the stall
//! watchdog sees the suspended parent). The default duration is unlimited;
//! `max_duration_secs` overrides per call. The deadline is managed *here*,
//! not by the dispatcher's outer timeout wrapper — that wrapper drops its
//! future on expiry, which would orphan the child. On deadline expiry this
//! tool cancels the child itself.

use std::time::Duration;

use kameo::actor::Spawn;

use crate::feat::chat_input::protocol::command::EnqueueUserMessage;
use crate::feat::provider::protocol::command::CancelStream;
use crate::feat::session::chat_entry::{ChatEntry, ChatEntryKind};
use crate::feat::session::chat_session::ChatSessionState;
use crate::feat::session::model_selection::ModelSelection;
use crate::feat::session_lifecycle::protocol::event::SessionCreated;
use crate::feat::tools_actor::BoxedToolFuture;
use crate::feat::tools_actor::task_phase_listener_actor::{
    TaskPhaseListenerActor, TaskPhaseListenerDeps,
};
use crate::feat::tools_actor::tool_types::{ToolCall, ToolContext, ToolDefinition, ToolResult};
use crate::protocol::SessionId;

/// The `task` tool's registration name, shared by the registry and the
/// depth-1 assembly filter.
pub const TASK_TOOL_NAME: &str = "task";

/// Returns the tool definition for `task`.
pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: TASK_TOOL_NAME.to_owned(),
        description: "Delegate a task to a fresh subagent session and block until it finishes. \
            Spawns a new session inheriting your model, cwd, tools, skills, and MCP servers — \
            but with empty history: it sees only the prompt you give it. When its session goes \
            idle, its final chat entry is returned as this tool's result. \
            \
            WHEN TO USE: open-ended search or exploration where the first try may miss; \
            multi-step sub-tasks whose intermediate tool output you don't need; \
            independent sub-tasks in parallel. \
            \
            WHEN NOT TO USE: reading a specific file or symbol (use read/grep directly); \
            one obvious tool call; tasks that depend on each other's results. \
            \
            Usage notes: \
            (1) The prompt must be self-contained — the subagent cannot see this conversation \
            or the user's intent; say whether it should research or make changes. \
            (2) Launch independent tasks concurrently — multiple task calls in one message. \
            (3) The user can watch and steer the subagent live; cancelling it returns the \
            cancel to you as the result. \
            (4) Only the final message comes back; intermediate work stays in the subagent's \
            session. \
            \
            TIMEOUT: unlimited by default. \
            Pass `max_duration_secs` to bound the subagent; on expiry the subagent is \
            cancelled and a failure is returned."
            .to_owned(),
        prompt_snippet: Some(
            "Spawn a subagent session for a self-contained sub-task and await its result"
                .to_owned(),
        ),
        prompt_guidelines: vec![
            "Prefer task for open-ended exploration; keep focused lookups (read/grep) in \
            your own context."
                .to_owned(),
            "Write the prompt as a complete brief: goal, constraints, and whether to \
            research or make changes. Include a description so the user can follow along \
            in the sidebar."
                .to_owned(),
        ],
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "Self-contained brief for the subagent. It sees nothing \
                    else — no conversation history, no user intent."
                },
                "description": {
                    "type": "string",
                    "description": "A 3-5 word summary of the task, shown as the subagent \
                    session's title in the sidebar."
                },
                "model": {
                    "type": "string",
                    "description": "Optional model id for the subagent. Defaults to this \
                    session's model."
                },
                "max_duration_secs": {
                    "type": "number",
                    "description": "Maximum duration in seconds to wait for the subagent. \
                    Unlimited by default; 0 also means unlimited. On expiry the subagent \
                    session is cancelled and a failure is returned."
                }
            },
            "required": ["prompt"]
        }),
        server_tool_type: None,
    }
}

/// Parsed `task` tool arguments.
struct TaskArgs {
    /// The self-contained brief for the subagent.
    prompt: String,
    /// Optional 3-5 word title for the child session.
    description: Option<String>,
    /// Optional model override.
    model: Option<String>,
    /// Optional wait budget in seconds; `None` (or 0) means unlimited.
    max_duration_secs: Option<u64>,
}

/// Parses the tool call arguments.
fn parse_args(raw: &str) -> Result<TaskArgs, String> {
    let v: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| format!("invalid JSON arguments: {e}"))?;
    let prompt = v
        .get("prompt")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .trim()
        .to_owned();
    if prompt.is_empty() {
        return Err("prompt is empty; provide a self-contained brief for the subagent".to_owned());
    }
    let model = v
        .get("model")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .map(str::to_owned);
    let description = v
        .get("description")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(str::to_owned);
    let max_duration_secs = super::extract_max_duration(raw);
    Ok(TaskArgs {
        prompt,
        description,
        model,
        max_duration_secs,
    })
}

/// Executes the `task` tool.
pub fn execute(call: ToolCall, ctx: ToolContext) -> BoxedToolFuture {
    Box::pin(async move { run(call, ctx).await })
}

/// How the awaited child run ended.
enum ChildOutcome {
    /// The child reached `Idle` — read its last entry.
    Finished,
    /// The wait budget expired — the child was cancelled.
    TimedOut(u64),
    /// The completion channel closed without a signal (listener died).
    ListenerGone,
}

/// Builds the child session linked to `parent_id` with inherited config.
fn build_child(
    parent: &ChatSessionState,
    parent_id: &SessionId,
    args: &TaskArgs,
    app_home: &std::path::Path,
) -> ChatSessionState {
    let mut child = ChatSessionState::new_child(parent_id, true);
    let profile = parent.profile();
    let model = args
        .model
        .clone()
        .map_or_else(|| profile.model.clone(), ModelSelection::Single);
    {
        let p = child.profile_mut();
        p.model = model;
        p.persona_name.clone_from(&profile.persona_name);
        p.reasoning_effort = profile.reasoning_effort;
        p.endpoint.clone_from(&profile.endpoint);
        p.disabled_tools.clone_from(&profile.disabled_tools);
        p.disabled_skills.clone_from(&profile.disabled_skills);
    }
    child.set_cwd(parent.cwd().to_path_buf());
    // Home resolves fresh at creation in every other path (runtime-only,
    // not persisted); the `task` tool's ctx carries the app paths.
    child.set_home(app_home.to_path_buf());
    child.set_enabled_mcp_servers(parent.enabled_mcp_servers().clone());
    let title = args
        .description
        .clone()
        .unwrap_or_else(|| title_from_prompt(&args.prompt));
    child.set_title(title);
    // Persistable: the child has been meaningfully created even though no
    // user keystrokes landed in it. Without this the child vanishes from
    // disk on archive.
    child.mark_interacted();
    child
}

/// Derives a fallback session title from the first line of the prompt.
fn title_from_prompt(prompt: &str) -> String {
    prompt
        .lines()
        .next()
        .unwrap_or("subagent task")
        .chars()
        .take(40)
        .collect()
}

/// Result of the await step.
async fn await_child(
    bus: &crate::common::services::bus_service::BusService,
    completion: tokio::sync::oneshot::Receiver<()>,
    child_id: SessionId,
    deadline: Option<Duration>,
) -> ChildOutcome {
    let wait = async {
        // `send` fails only if the listener died; closing without a signal
        // is itself a signal of abnormal termination.
        match completion.await {
            Ok(()) => ChildOutcome::Finished,
            Err(_) => ChildOutcome::ListenerGone,
        }
    };
    match deadline {
        None => wait.await,
        Some(budget) => match tokio::time::timeout(budget, wait).await {
            Ok(outcome) => outcome,
            Err(_) => {
                bus.publish(CancelStream {
                    session_id: child_id,
                })
                .await;
                ChildOutcome::TimedOut(budget.as_secs())
            }
        },
    }
}

/// Classifies the child's final entry into a forwarded tool result message
/// and success flag.
fn classify_final_entry(entry: &ChatEntry) -> (bool, String) {
    match &entry.kind {
        ChatEntryKind::Error(text) => (false, text.clone()),
        _ => (true, entry.text()),
    }
}

async fn run(call: ToolCall, ctx: ToolContext) -> ToolResult {
    // Fail fast on missing context, mirroring restart_mcp.
    let Some(state) = ctx.state else {
        return tool_error(call, "no application state available");
    };
    let Some(parent_id) = ctx.session_id else {
        return tool_error(call, "no session ID available");
    };
    let Some(bus) = ctx.bus else {
        return tool_error(call, "no message bus available");
    };
    let Some(session_cap) = ctx.session_cap else {
        return tool_error(call, "no session authority available");
    };
    let args = match parse_args(&call.arguments) {
        Ok(args) => args,
        Err(msg) => return tool_error(call, &msg),
    };
    let deadline = args
        .max_duration_secs
        .filter(|s| *s > 0)
        .map(Duration::from_secs);

    // Snapshot the parent and build the child under the read lock.
    let child = {
        let guard = state.read();
        let Some(parent) = guard.session.get(&parent_id) else {
            return tool_error(call, "parent session not found in state");
        };
        build_child(parent, &parent_id, &args, ctx.app_paths.home_dir())
    };
    let child_id = child.session_id().clone();

    // Insert the child before any event flies. Every later writer (session
    // actor on EnqueueUserMessage, MCP coordinator on SessionCreated) looks
    // the session up by id — insertion must precede publication or they
    // would each `get_or_create` a bare session over the real child.
    state.with_session(&session_cap, |view| {
        view.session.map().insert(child);
    });

    // Register the in-flight pair before blocking so the stall watchdog sees
    // the suspended parent. The guard covers success, failure, and abort
    // (parent tool-call future dropped) paths.
    let registry = ctx.task_spawns.clone().unwrap_or_default();
    let guard = registry.guard(parent_id.clone(), child_id.clone());

    // Listener before publication: guarantees the Idle subscription exists
    // before SessionCreated/EnqueueUserMessage can trigger any phase change.
    let (completion_tx, completion_rx) = tokio::sync::oneshot::channel();
    let listener = TaskPhaseListenerActor::spawn(TaskPhaseListenerDeps {
        bus: bus.clone(),
        child_id: child_id.clone(),
        completion: completion_tx,
    });
    listener.wait_for_startup().await;

    // Publish: lifecycle actors react to SessionCreated (MCP reconcile,
    // scans, persistence); the session actor turns EnqueueUserMessage into
    // the child's first dispatch.
    bus.publish(SessionCreated {
        session_id: child_id.clone(),
    })
    .await;
    bus.publish(EnqueueUserMessage {
        session_id: child_id.clone(),
        entry: ChatEntry::user(args.prompt.clone()),
    })
    .await;

    let outcome = await_child(&bus, completion_rx, child_id.clone(), deadline).await;
    guard.defuse();

    match outcome {
        ChildOutcome::Finished => {
            let (success, content) = {
                let snapshot = state.read();
                snapshot
                    .session
                    .get(&child_id)
                    .and_then(|child| child.history().last())
                    .map_or_else(
                        || (false, "subagent produced no output".to_owned()),
                        classify_final_entry,
                    )
            };
            forward(call, success, content)
        }
        ChildOutcome::TimedOut(secs) => forward(
            call,
            false,
            format!(
                "Subagent task timed out after {secs}s and was cancelled. \
                 Retry with a larger max_duration_secs value, or split the task."
            ),
        ),
        ChildOutcome::ListenerGone => forward(
            call,
            false,
            "Subagent completion listener terminated unexpectedly.".to_owned(),
        ),
    }
}

/// Builds the final [`ToolResult`].
fn forward(call: ToolCall, success: bool, content: String) -> ToolResult {
    ToolResult {
        tool_call_id: call.id,
        name: call.name,
        content,
        success,
        full_content: None,
        truncation: None,
        pin_position: None,
    }
}

fn tool_error(call: ToolCall, msg: &str) -> ToolResult {
    forward(call, false, format!("Error: {msg}"))
}
