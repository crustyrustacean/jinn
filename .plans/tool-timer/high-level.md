# Tool Streaming (Bash) — High-Level Plan

## Problem

When the bash tool executes a long-running command, the user sees nothing until it completes. The ToolCall entry shows the command, but the ToolResult entry doesn't exist until execution finishes. For slow commands (builds, test suites, long-running scripts), the user has no visibility into what's happening.

## Proposed Solution

Re-architect bash tool execution to stream stdout/stderr line-by-line in real time into the chat log. The ToolResult entry is created early (when execution starts) with a neutral background, and content is appended incrementally as output arrives. When execution finishes, the entry is finalized with green (success) or red (failure) background.

### Architecture Overview

The change spans four areas:

1. **New events on the bus** — `ToolExecutionStarted` (tool began running) and `ToolExecutionOutput` (incremental stdout/stderr line).
2. **Early ToolResult entry** — Session actor creates the ToolResult entry on `ToolExecutionStarted` instead of waiting for completion. Content grows via `ToolExecutionOutput`. Background starts neutral, becomes green/red on completion.
3. **Bash execution rewrite** — Replace `.output()` (wait for completion) with `tokio::process::Command::spawn()` + line-by-line stdout/stderr reading. Each line emits a `ToolExecutionOutput` event.
4. **Rendering adjustments** — ToolResult renderer handles neutral background for pending state. Content fingerprint excludes the growing content to keep the line count cache stable.

### Key Design Decisions

- **Only bash gets streaming.** All other tools (echo, read, write, get_time, skill) remain fire-and-forget — they're instant.
- **Neutral background for pending ToolResult.** The user explicitly requested: "color the background neutral and then change it based on success/failure." A new `ToolResultStatus` enum (`Pending`, `Success`, `Failure`) drives the background color.
- **`ToolExecutionOutput` carries partial content (deltas).** Each event carries new lines of output, not the accumulated total. The session actor appends to the existing ToolResult entry's content. This mirrors the `StreamToken` pattern used for LLM streaming.
- **Fingerprint excludes content for pending entries.** The `content_fingerprint()` hash for a `ToolResult` in `Pending` state excludes the growing content field. This prevents the line count cache from invalidating on every output line (the line count changes naturally as content grows, but the cache key stays stable for the fingerprint check). Instead, the fingerprint for pending entries only hashes `id + name + status`.
- **Non-streaming tools are unchanged.** Tools that don't emit `ToolExecutionStarted` continue to create ToolResult entries on `ToolExecutionCompleted` as before — no early creation, no neutral background, just the final result with success/failure.

---

## Phases

- [x] Phase 1: Data model changes — new events, ToolResult status enum, serialization backward compat
- [x] Phase 2: Bash execution rewrite — spawn process, read stdout/stderr incrementally, emit streaming events
- [x] Phase 3: Session actor & rendering — early ToolResult creation, incremental content append, neutral background, finalization
- [x] Phase 4: Tests & polish — unit tests for new events, integration tests for streaming flow, edge cases

---

## Phase 1: Data Model Changes

### Goal

Add the types needed for streaming tool execution: new bus events, a `status` field on `ChatEntryKind::ToolResult`, and backward-compatible serialization.

### Details

**New events** (in `crates/nullslop-domain/src/feat/tools_actor/protocol/event.rs`):

- `ToolExecutionStarted { session_id: SessionId, tool_call_id: String, name: String }` — emitted when the tool begins executing (after arguments are complete, before output).
- `ToolExecutionOutput { session_id: SessionId, tool_call_id: String, output: String }` — emitted for each chunk of stdout/stderr. `output` is a delta (new lines), not accumulated.

Both must be registered in the `Event` enum in `crates/nullslop-domain/src/protocol/app_msg/event.rs` with `type_name()` match arms.

**ToolResult status** — Add a `status: ToolResultStatus` field to `ChatEntryKind::ToolResult`:

```rust
/// Execution status of a tool result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ToolResultStatus {
    /// Tool is still executing — content may grow.
    Pending,
    /// Tool completed successfully.
    Success,
    /// Tool failed.
    Failure,
}
```

The existing `success: bool` field is replaced by `status`. This affects:
- `ChatEntryKind::ToolResult` variant definition
- `ChatEntry::tool_result()` constructor
- All match arms on `ToolResult`
- Serialization/deserialization (must handle old `success: bool` format for backward compat)
- `text()`, `content_fingerprint()`, `kind_str()` methods

**Session actor subscriptions** — Subscribe to `ToolExecutionStarted` and `ToolExecutionOutput` in the session actor.

**Acceptance Criteria:**

- [ ] `ToolExecutionStarted` and `ToolExecutionOutput` event structs exist with `#[derive(EventMsg)]`
- [ ] Both events are registered as variants in the `Event` enum with `type_name()` arms
- [ ] `ToolResultStatus` enum exists (Pending, Success, Failure) with Serialize/Deserialize
- [ ] `ChatEntryKind::ToolResult` has `status: ToolResultStatus` instead of `success: bool`
- [ ] `ChatEntry::tool_result()` updated to accept `ToolResultStatus`
- [ ] All existing match arms on `ChatEntryKind::ToolResult` updated
- [ ] `text()`, `content_fingerprint()`, `kind_str()` updated for new field
- [ ] Serialization backward compat: old JSON with `success: bool` deserializes correctly (maps to Success/Failure)
- [ ] Session actor subscribes to new events
- [ ] `just check` passes

---

## Phase 2: Bash Execution Rewrite

### Goal

Rewrite the bash builtin to spawn a process and stream stdout/stderr line-by-line instead of waiting for completion with `.output()`.

### Details

The current `builtin_bash::execute` uses `tokio::process::Command::new().output()` which waits for the process to finish. It needs to:

1. Spawn the process with `stdout(Stdio::piped())` and `stderr(Stdio::piped())`.
2. Emit `ToolExecutionStarted` via the sink.
3. Read stdout and stderr concurrently (tokio `select!` or `join!`), line-by-line via `BufReader::lines()`.
4. For each line, emit `ToolExecutionOutput` with the line content (including a newline).
5. Wait for the process to exit, then emit `ToolExecutionCompleted` with final content (truncated) and success/failure based on exit code.

The `execute` function signature must change — it needs access to `session_id` and the `MessageSink` to emit events. Currently it only receives `(ToolCall, ToolContext)`. Options:

- **Add fields to `ToolContext`**: `session_id` is already there (Option), add `sink: Option<Arc<dyn MessageSink>>`. This keeps the function signature the same.
- The orchestrator already builds `ToolContext` per-call and has access to the sink via `ctx.sink()`.

The existing builtin tools return `ToolResult` from their `execute` function. For streaming, the bash tool still needs to return a `ToolResult` (the orchestrator expects this for `ToolExecutionCompleted`). But the content can be a summary or truncated version since the full output was already streamed.

**Key concern**: The orchestrator's `dispatch_tool_call` spawns a tokio task and sends `ToolExecutionCompleted` when the future resolves. For streaming bash, we need the spawned task to also emit `ToolExecutionStarted` and `ToolExecutionOutput` before the final `ToolExecutionCompleted`. The sink is already cloned into the spawned task, so this works.

**Acceptance Criteria:**

- [ ] `builtin_bash::execute` spawns process with piped stdout/stderr
- [ ] `ToolExecutionStarted` emitted before process starts
- [ ] `ToolExecutionOutput` emitted for each stdout/stderr line
- [ ] `ToolExecutionCompleted` emitted after process exits with correct success/failure
- [ ] Timeout handling preserved (kill process on timeout, emit failure)
- [ ] Output truncation logic preserved (last 2000 lines / 50KB)
- [ ] Non-streaming builtins unchanged
- [ ] `just check` passes
- [ ] Existing bash tests updated and pass

---

## Phase 3: Session Actor & Rendering

### Goal

Wire up the session actor to create ToolResult entries early on `ToolExecutionStarted`, append content on `ToolExecutionOutput`, and finalize on `ToolExecutionCompleted`. Update the tool result renderer to show neutral background for pending entries.

### Details

**Session actor handlers** (in `crates/nullslop-domain/src/feat/session/session_actor/handlers/event.rs`):

- `on_tool_execution_started`: Create a `ChatEntry::tool_result()` with `status: ToolResultStatus::Pending`, empty content, and push to history. Track the entry index in a new ephemeral field (similar to `streaming_tool_call_indices`).
- `on_tool_execution_output`: Find the pending ToolResult entry by `tool_call_id`, append `output` to its content.
- `on_tool_execution_completed`: Find the pending ToolResult entry, set final content (truncation summary if needed), set `status` to Success/Failure based on result. Save session.

**Ephemeral tracking** (in `SessionCoreEphemeral`):

- Add `streaming_tool_result_indices: HashMap<String, usize>` — maps `tool_call_id` to history index. Cleared on stream cancel/finish.

**Tool result renderer** (in `crates/nullslop-domain/src/feat/ui/chat_log/tool_result.rs`):

- Map `ToolResultStatus` to background color:
  - `Pending` → neutral/dark gray (a new theme field or a reasonable default)
  - `Success` → existing `tool_success_bg`
  - `Failure` → existing `tool_failure_bg`
- The renderer already handles content with newlines — no structural changes needed.

**Content fingerprint** (in `ChatEntry::content_fingerprint()`):

- For `ToolResult` in `Pending` status, only hash `id + name + status` (not content). This prevents cache invalidation on every output line.
- For `Success`/`Failure`, hash everything as normal (content is final).

**Tool orchestrator** — For non-streaming tools, no change. They don't emit `ToolExecutionStarted`, so the session actor doesn't create an early ToolResult. The existing `on_tool_execution_completed` path still works for them.

**Acceptance Criteria:**

- [ ] `on_tool_execution_started` creates pending ToolResult entry
- [ ] `on_tool_execution_output` appends content to pending entry
- [ ] `on_tool_execution_completed` finalizes entry with success/failure status
- [ ] Pending ToolResult entries render with neutral background
- [ ] Completed ToolResult entries render with green/red background as before
- [ ] Non-streaming tools create ToolResult entries on completion as before
- [ ] `streaming_tool_result_indices` tracked in ephemeral state, cleared on finish/cancel
- [ ] Content fingerprint stable for pending entries (excludes growing content)
- [ ] `just check` passes
- [ ] `just test` passes

---

## Phase 4: Tests & Polish

### Goal

Comprehensive tests for the new streaming flow, edge cases, and backward compatibility.

### Details

**Unit tests:**

- `ToolExecutionStarted` / `ToolExecutionOutput` serialization roundtrip
- `ToolResultStatus` serialization (including backward compat with old `success: bool` JSON)
- `content_fingerprint()` stability for pending ToolResult entries
- Session actor handlers: early creation, content append, finalization

**Integration tests:**

- Full bash streaming flow: ToolUseStarted → ToolCallReceived → ToolExecutionStarted → (multiple ToolExecutionOutput) → ToolExecutionCompleted
- Non-streaming tool flow unchanged (echo, read, write)
- Cancellation mid-stream (cancel streaming while tool is running)
- Empty output (command produces nothing)
- Mixed stdout/stderr output
- Timeout during streaming

**Acceptance Criteria:**

- [ ] All new events serialize/deserialize correctly
- [ ] Old session files with `success: bool` load correctly
- [ ] Pending fingerprint is stable across content growth
- [ ] Full streaming flow test passes
- [ ] Non-streaming tools still work
- [ ] Cancellation cleans up pending entries
- [ ] `just test` passes
- [ ] `just lint` passes
