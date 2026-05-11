# Phase 1: Split `nsslice-tools/src/lib.rs`

## Problem

`lib.rs` in `nsslice-tools` is 1496 lines. It contains the tool orchestrator actor (types, dispatch, handlers) plus 4 built-in tool definitions with their execute functions. The built-in tools are a separable concern — they're a static list consumed by the actor at activation.

## What Moves / What Stays

**Moves to `src/builtin.rs`:**
- `BuiltinToolEntry` type alias
- `builtin_tools()` function
- `echo_definition()` + `execute_echo()`
- `get_time_definition()` + `execute_get_time()`
- `file_read_definition()` + `execute_file_read()`
- `file_write_definition()` + `execute_file_write()`

**Stays in `lib.rs`:**
- `BoxedToolFuture` type alias
- `ToolRegistration` enum + Debug impl
- `PendingBatch` struct
- `spawn()` function
- `ToolOrchestratorDirectMsg` enum
- `ToolOrchestratorActor` struct + Actor impl + handler methods
- `#[cfg(test)] mod tests`

## File Changes

1. **Create `crates/slices/nsslice-tools/src/builtin.rs`** — contains all built-in tool code extracted from lib.rs. Needs its own imports for `ToolDefinition`, `ToolCall`, `ToolResult`, `BoxedToolFuture`.

2. **Modify `crates/slices/nsslice-tools/src/lib.rs`** — remove builtin tool code, add `mod builtin;`, use `builtin::BuiltinToolEntry` in `activate()`.

## Implementation Order

1. Create `builtin.rs` with the extracted code
2. Update `lib.rs` to use the new module
3. Run `just check` to verify compilation

## Acceptance Criteria

- [x] `crates/slices/nsslice-tools/src/builtin.rs` exists with all 4 builtin tool definitions + execute functions
- [x] `lib.rs` has `mod builtin;` and no longer contains `echo_definition`, `execute_echo`, `get_time_definition`, `execute_get_time`, `file_read_definition`, `execute_file_read`, `file_write_definition`, `execute_file_write`, `builtin_tools`, `BuiltinToolEntry`
- [x] `just check` passes
- [x] `just test` passes
- [x] `just lint` passes

---

## Review: Phase 1 — Split nsslice-tools lib.rs

### Changes

Extracted all 4 built-in tool definitions and their execute functions into `src/builtin.rs`. The `BuiltinToolEntry` type alias and `builtin_tools()` registry function also moved. The actor types, `spawn()`, and all handler methods remain in `lib.rs`.

### Divergence Summary

- Needed `use builtin::{execute_echo, ...}` in lib.rs so the test module's `use super::*` can find the execute functions. Without this, `pub(super)` on the builtin functions makes them accessible to lib.rs but not visible via glob import in the nested test module.
- `BuiltinToolEntry` was not imported in lib.rs — the type is only used inside `builtin_tools()` which is now in `builtin.rs`, so no re-export was needed.

### Verification

- `just check` — clean
- `just test` — all pass
- `just lint` — pass (only pre-existing test-length warnings)

### Risks

None. Pure internal reorganization.

### Next Steps

Phase 2: Split `nsslice-llm/src/lib.rs`.
