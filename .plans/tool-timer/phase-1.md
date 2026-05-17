# Phase 1: Data Model Changes — Execution Plan

## Problem

We need new bus events (`ToolExecutionStarted`, `ToolExecutionOutput`) and a reworked `ToolResult` chat entry kind that supports a "pending" state. These types must exist before the bash execution rewrite (Phase 2) and the session actor wiring (Phase 3) can proceed.

## What Moves / What Stays

### Moves
- `ChatEntryKind::ToolResult { id, name, content, success: bool }` → `ChatEntryKind::ToolResult { id, name, content, status: ToolResultStatus }`

### New
- `ToolExecutionStarted` event struct
- `ToolExecutionOutput` event struct
- `ToolResultStatus` enum (Pending, Success, Failure)
- Two new `Event` enum variants
- Two new event subscriptions in session actor
- Two new match arms in session actor event handler

### Stays
- All other `ChatEntryKind` variants (unchanged)
- `ToolResult` struct in `nullslop-provider` (unchanged — this is the execution result, not the chat entry)
- All other events, commands, intents
- All rendering code (Phase 3)
- All tool execution code (Phase 2)

## File Changes

### 1. CREATE `crates/nullslop-domain/src/feat/session/tool_result_status.rs`

New enum:

```rust
//! Execution status for tool result entries.

use serde::{Deserialize, Serialize};

/// Execution status of a tool result entry.
///
/// Controls the background color of the rendered entry and whether
/// content is still growing (streaming).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ToolResultStatus {
    /// Tool is still executing — content may grow incrementally.
    Pending,
    /// Tool completed successfully.
    Success,
    /// Tool failed.
    Failure,
}
```

### 2. MODIFY `crates/nullslop-domain/src/feat/session/mod.rs`

Add `pub(crate) mod tool_result_status;`.

### 3. MODIFY `crates/nullslop-domain/src/feat/session/chat_entry.rs`

- Import `ToolResultStatus`
- Change `ChatEntryKind::ToolResult` from `{ id, name, content, success: bool }` to `{ id, name, content, status: ToolResultStatus }`
- Update `ChatEntry::tool_result()` signature: replace `success: bool` with `status: ToolResultStatus`
- Update `text()` match arm for ToolResult
- Update `content_fingerprint()` match arm for ToolResult
- Update `kind_str()` — already returns `"tool_result"`, no change needed
- Update `Serialize` impl for `ChatEntryKind::ToolResult` — serialize `status` instead of `success`
- Update `Deserialize` impl for `ChatEntryKind::ToolResult` — handle both old (`success: bool`) and new (`status: ToolResultStatus`) formats for backward compat
- Update `is_pinnable()` — currently matches `ToolResult { .. }`, no change needed

### 4. MODIFY `crates/nullslop-domain/src/feat/tools_actor/protocol/event.rs`

Add two new event structs:

```rust
/// A tool has started executing.
///
/// Emitted by the tool orchestrator when a tool begins actual execution
/// (after arguments are complete). The session actor creates a pending
/// ToolResult entry. Only emitted for streaming tools (e.g., bash).
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("tool")]
pub struct ToolExecutionStarted {
    /// The session this execution belongs to.
    pub session_id: SessionId,
    /// The unique ID for this tool call.
    pub tool_call_id: String,
    /// The name of the tool being executed.
    pub name: String,
}

/// Incremental output from a running tool.
///
/// Emitted by streaming tools as they produce output. Each event carries
/// a delta (new lines), not the accumulated total. The session actor
/// appends to the pending ToolResult entry's content.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("tool")]
pub struct ToolExecutionOutput {
    /// The session this output belongs to.
    pub session_id: SessionId,
    /// The tool call ID this output is for.
    pub tool_call_id: String,
    /// New output text (delta, not accumulated).
    pub output: String,
}
```

### 5. MODIFY `crates/nullslop-domain/src/protocol/app_msg/event.rs`

- Import `ToolExecutionStarted`, `ToolExecutionOutput`
- Add `Event::ToolExecutionStarted(ToolExecutionStarted)` variant
- Add `Event::ToolExecutionOutput(ToolExecutionOutput)` variant
- Add `type_name()` match arms for both

### 6. MODIFY `crates/nullslop-domain/src/lib.rs`

- Update re-exports to include `ToolExecutionStarted`, `ToolExecutionOutput`

### 7. MODIFY `crates/nullslop-domain/src/protocol.rs`

- Update re-exports to include `ToolExecutionStarted`, `ToolExecutionOutput`

### 8. MODIFY `crates/nullslop-domain/src/feat/session/session_actor.rs`

- Import new events
- Subscribe to `ToolExecutionStarted` and `ToolExecutionOutput`
- Add stub handlers in the event dispatch match arm (just log/ignore for now — Phase 3 implements them)

### 9. MODIFY `crates/nullslop-domain/src/feat/session/session_actor/handlers/event.rs`

- Import new events
- Add handler method stubs for `on_tool_execution_started` and `on_tool_execution_output`
- Update `on_tool_execution_completed` to use `ToolResultStatus` instead of `success: bool`

### 10. MODIFY all consumers of `ChatEntryKind::ToolResult` with `success` field

Find and update all match arms / constructors:
- `crates/nullslop-domain/src/feat/session/chat_entry_tests.rs` — update `tool_result_entry_has_tool_result_kind` and all `ChatEntry::tool_result()` calls
- `crates/nullslop-domain/src/feat/session/chat_session/chat_session_tests.rs` — update any `ToolResult { success, .. }` matches
- `crates/nullslop-domain/src/feat/session/session_store/sqlite_tests.rs` — update test entries
- `crates/nullslop-domain/src/feat/ui/chat_log/tool_result.rs` — renderer: `to_lines()` changes `success: bool` param to `status: ToolResultStatus`
- `crates/nullslop-domain/src/feat/ui/chat_log/tool_result.rs` tests — update `render_context` calls
- `crates/nullslop-domain/src/feat/ui/chat_log/renderer.rs` — `entry_to_lines()` passes `*status` instead of `*success`
- `crates/nullslop-domain/src/feat/ui/chat_log/tool_call.rs` — no change (tool_call, not tool_result)
- `crates/nullslop-domain/src/feat/ui/sidebar/pins/pins_section.rs` — if it matches on ToolResult
- `crates/nullslop-domain/src/feat/ui/status_bar/turn_counter.rs` — if it matches on ToolResult
- `crates/nullslop-domain/src/feat/context/strategy/token_estimator.rs` — if it matches on ToolResult
- `crates/nullslop-domain/src/feat/provider/entries_to_messages.rs` — if it matches on ToolResult
- `crates/nullslop-domain/src/feat/chat_entry_selection/validator.rs` — if it matches on ToolResult

### 11. MODIFY `crates/nullslop-domain/src/feat/tools_actor/tools_actor_tests.rs`

- Update any ToolResult assertions

## Implementation Order

1. Create `ToolResultStatus` enum
2. Register module in `session/mod.rs`
3. Update `ChatEntryKind::ToolResult` definition and all `chat_entry.rs` methods (text, fingerprint, serialize, deserialize)
4. Update `ChatEntry::tool_result()` constructor
5. Add new event structs in `tools_actor/protocol/event.rs`
6. Register events in `Event` enum
7. Update re-exports (`lib.rs`, `protocol.rs`)
8. Update session actor subscriptions and handler stubs
9. Update `on_tool_execution_completed` handler
10. Update all consumers (renderers, tests, match arms)
11. Run `just check`

## Acceptance Criteria

- [x] `ToolResultStatus` enum exists in `crates/nullslop-domain/src/feat/session/tool_result_status.rs` with Pending, Success, Failure variants
- [x] `ChatEntryKind::ToolResult` uses `status: ToolResultStatus` instead of `success: bool`
- [x] `ChatEntry::tool_result()` accepts `ToolResultStatus`
- [x] `ToolExecutionStarted` event exists with `session_id`, `tool_call_id`, `name`
- [x] `ToolExecutionOutput` event exists with `session_id`, `tool_call_id`, `output`
- [x] Both new events registered in `Event` enum with `type_name()` arms
- [x] Both new events re-exported from `lib.rs` and `protocol.rs`
- [x] Session actor subscribes to both new events
- [x] Deserialization backward compat: old JSON with `success: bool` maps to Success/Failure
- [x] All existing tests pass (after updating to new API)
- [x] `just check` passes

---

## Review: Phase 1 — Data Model Changes

### Changes

- Created `ToolResultStatus` enum (Pending, Success, Failure) in a new module
- Replaced `success: bool` with `status: ToolResultStatus` on `ChatEntryKind::ToolResult`
- Added `ToolExecutionStarted` and `ToolExecutionOutput` event structs
- Registered both events in the `Event` enum with `type_name()` arms
- Added `tool_pending_bg` color to `Theme`, `ThemeFile`, and `themes/default.toml` (dark gray-blue `[45, 45, 50]`)
- Updated tool result renderer to use `ToolResultStatus` for background selection
- Added backward-compatible deserialization (old `success: bool` → `ToolResultStatus::Success/Failure`)
- Added session actor subscriptions and stub handlers for new events
- Updated all 9 consumer files (tests, renderers, match arms)

### Divergence Summary

- The `provider::ToolResult` struct (in `nullslop-provider`) was NOT changed — it still uses `success: bool`. The `ToolResultStatus` enum only applies to `ChatEntryKind::ToolResult`. The session actor handler maps between them.
- The theme file in `themes/default.toml` needed updating for the new `tool_pending_bg` field — this wasn't in the original plan but was necessary since the default theme is parsed from TOML.
- The backward compat deserialization uses `serde_json::Value` intermediate to try new format first, then fall back to old — this is more robust than a single struct with optional fields.

### Verification

- `just check` passes (1 pre-existing warning about unused variable)
- `just test` passes (all unit tests, integration tests, and e2e cucumber tests)

### Risks

- The backward compat deserialization uses `serde_json::Value` as intermediate — if the JSON payload is very large this could be inefficient. For tool results this is fine.
- The theme file change (`tool_pending_bg`) is additive so existing custom themes will fall back to the `Color::Reset` default from `resolve_standalone`. This means users without the new field will get a Reset background for pending entries. We may want to address this in Phase 3 by using a sensible fallback.

### Next Steps

Phase 2: Bash execution rewrite — spawn process with piped stdout/stderr, emit `ToolExecutionStarted`/`ToolExecutionOutput`/`ToolExecutionCompleted` events.
