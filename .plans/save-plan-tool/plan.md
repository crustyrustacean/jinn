# save_plan Tool — Context-Rich Specification

## Problem

When an agent saves a plan file (e.g., `.plans/my-task/plan.md`), the resulting chat entry is ephemeral — it scrolls out of context during compaction. The agent loses sight of the plan it committed to, leading to drift. We need a tool that writes a plan file _and_ pins the resulting chat entry so it survives compaction.

## Solution

Create a new `save_plan` built-in tool alongside `write` in `crates/jinn-domain/src/feat/tools_actor/`. It takes the same `path` + `content` parameters as `write`, writes the file identically, but sets `pin_position: Some(ToolResultPinPosition::Relative)` on the success result. The tool's description and guidelines encourage the `.plans/<task>/` path convention. The result content reports the file path and byte count — not the plan body.

---

## Dialectical Outcomes (Why)

### Pin position: `Relative`
**Decision:** Use `PinPosition::Relative` so the plan stays at its original position in history and survives compaction.

**Alternatives rejected:**
- `Top` — forces the plan to the top of every prompt like a system instruction. Disorienting if multiple plans are saved across a session.
- `Bottom` — forces the plan just before the current message. Useful for visibility but changes the narrative flow of the conversation.

**Rationale:** `Relative` mirrors the `skill` tool's proven approach. The plan appears in chronological context and survives compaction — exactly what's needed.

### New standalone tool vs. extending `write`
**Decision:** Create a separate `save_plan` tool with its own name, description, and guidelines.

**Alternatives rejected:**
- Adding an optional `pin` parameter to `write` — makes `write` more general-purpose but gives the LLM more rope. A dedicated tool has clearer semantics and lets us iterate independently. If agents don't use it properly, the `write` extension path remains open.

**Rationale:** Semantic clarity for the LLM. A tool called `save_plan` with `.plans/`-specific guidelines is more discoverable than a `write` call with a magic boolean flag.

### Path convention: soft encouragement, no enforcement
**Decision:** The tool description and `prompt_guidelines` suggest `.plans/<task>/` but do not validate the path.

**Alternatives rejected:**
- No mention — too easy for the LLM to forget the convention.
- Hard enforcement (reject non-`.plans/` paths) — too rigid. Plans might live elsewhere in legitimate workflows.

### Result content: path + byte count, not plan body
**Decision:** Return `"saved plan to <path> (<N> bytes)"` — not the full plan text.

**Alternatives rejected:**
- Returning full content like `skill` does — wastes tokens. The plan is already on disk and the pin is about the _entry_ persisting, not the _content_ being re-read.

---

## Relevant Files (Where)

| File | Action |
|------|--------|
| `crates/jinn-domain/src/feat/tools_actor/save_plan.rs` | **Create** — new tool module |
| `crates/jinn-domain/src/feat/tools_actor/registry.rs` | **Modify** — register the new tool |
| `crates/jinn-domain/src/feat/tools_actor/mod.rs` | **Modify** — add `pub mod save_plan;` |

---

## Key Code Context (What)

### `ToolResult` struct (from `jinn-provider/src/tool_types.rs`)

The struct that every tool's `execute()` returns. The key field is `pin_position`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolResult {
    pub tool_call_id: String,
    pub name: String,
    pub content: String,
    pub success: bool,
    pub full_content: Option<String>,
    pub truncation: Option<TruncationMeta>,
    pub pin_position: Option<ToolResultPinPosition>,
}
```

### `ToolResultPinPosition` enum (from `jinn-provider/src/tool_types.rs`)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolResultPinPosition {
    Top,
    Bottom,
    Relative,
}
```

### `ToolDefinition` struct (from `jinn-provider/src/tool_types.rs`)

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    pub prompt_snippet: Option<String>,
    pub prompt_guidelines: Vec<String>,
    pub server_tool_type: Option<ServerToolType>,
}
```

### `write::execute` (the reference implementation to mirror)

The `write` tool in `crates/jinn-domain/src/feat/tools_actor/write.rs` is the structural template. It:
1. Parses `path` + `content` from JSON arguments via `parse_args`.
2. Resolves the path against `ctx.cwd` via `resolve_path`.
3. Creates parent directories with `tokio::fs::create_dir_all`.
4. Writes the file with `tokio::fs::write`.
5. Returns a `ToolResult` with `pin_position: None`.

The `save_plan` tool follows the identical flow, differing only in:
- Tool name, description, guidelines.
- `pin_position: Some(ToolResultPinPosition::Relative)` on success.
- Result message wording (`"saved plan to ..."` vs `"wrote ... bytes to ..."`).

### `skill::execute` (the pinning reference)

In `crates/jinn-domain/src/feat/tools_actor/skill.rs`, the skill tool sets:
```rust
pin_position: Some(ToolResultPinPosition::Relative),
```
This is the only existing builtin tool that sets a non-None `pin_position`.

### `registry.rs` — tool registration pattern

```rust
use super::{BoxedToolFuture, bash, edit, get_time, read, skill, write};

pub fn builtin_tools(bash_config: &BashConfig) -> Vec<BuiltinToolEntry> {
    let mut entries = vec![
        // ... other tools ...
        (
            write::definition(),
            write::execute as fn(ToolCall, ToolContext) -> BoxedToolFuture,
        ),
        // ... other tools ...
    ];
    entries.extend(todo_list::tools::tool_entries());
    entries
}
```

### `mod.rs` — module declaration pattern

```rust
pub mod bash;
pub mod edit;
pub mod get_time;
pub mod protocol;
pub mod read;
pub mod registry;
pub mod skill;
pub mod tool_entry;
pub mod tool_types;
pub(crate) mod truncation;
pub mod write;
```

---

## Implementation Algorithm (How)

### Phase 1: Create `save_plan.rs`

Create `crates/jinn-domain/src/feat/tools_actor/save_plan.rs` with:

1. **`definition()` function** returning `ToolDefinition`:
   - `name: "save_plan"`
   - `description`: explains it writes a plan file and pins the result. Mentions `.plans/` convention.
   - `prompt_snippet`: `"Save a plan file"`
   - `prompt_guidelines`: suggest using `.plans/<task-slug>/plan.md` path pattern.
   - `parameters`: JSON schema with `path` (string) and `content` (string), both required.
   - `server_tool_type: None`

2. **`execute()` function** with signature `fn(ToolCall, ToolContext) -> BoxedToolFuture`:
   - Parse `path` + `content` from `call.arguments` JSON via `parse_args`.
   - If parse fails, return error `ToolResult` with `pin_position: None`.
   - Resolve path: if relative, join with `ctx.cwd`; if absolute, use as-is. (Reuse `write::resolve_path` logic or duplicate inline — the function is a 5-line helper, not worth creating a shared dependency for.)
   - Create parent directories with `tokio::fs::create_dir_all`. On failure, return error `ToolResult`.
   - Write file with `tokio::fs::write`. On failure, return error `ToolResult`.
   - On success, return `ToolResult` with:
     - `content: format!("saved plan to {} ({} bytes)", resolved.display(), content.len())`
     - `success: true`
     - `pin_position: Some(ToolResultPinPosition::Relative)`

3. **`parse_args()` function**: identical to `write::parse_args` — extracts `path` and `content` strings from JSON.

### Phase 2: Register in `registry.rs`

1. Add `save_plan` to the `use super::{...}` import.
2. Add `(save_plan::definition(), save_plan::execute as fn(ToolCall, ToolContext) -> BoxedToolFuture)` to the `entries` vec in `builtin_tools()`.

### Phase 3: Declare module in `mod.rs`

Add `pub mod save_plan;` to the module declarations in alphabetical order (after `read`, before `skill`).

### Phase 4: Write tests

BDD-style tests within `#[cfg(test)] mod tests` in `save_plan.rs`. See Test Strategies below.

---

## Anti-Goals (Out of Scope)

- **No `write` tool modification.** The existing `write` tool is untouched.
- **No path validation.** The tool does not enforce `.plans/` prefix — it's a guideline only.
- **No idempotency checks.** Unlike `skill`, the tool does not check if a plan was already saved. Overwriting is intentional (plan updates).
- **No plan-specific formatting.** The tool does not parse, validate, or transform plan content. It writes whatever string it receives.
- **No truncation handling.** Plan files are expected to be small. The tool does not apply truncation to its output (the orchestrator handles truncation externally if needed).

---

## Edge Cases & Gotchas

1. **Empty path**: `parse_args` defaults to `""` when path is missing or non-string. Writing to an empty path will fail at `tokio::fs::write`, returning a clear OS error. This is acceptable — no special validation needed.

2. **Overwriting existing plans**: The tool intentionally overwrites. This is correct behavior — agents update plans.

3. **`pin_position` only on success**: Error results must have `pin_position: None`. Only the happy path pins.

4. **Module ordering**: In `mod.rs`, add `pub mod save_plan;` in alphabetical order (between `read` and `skill`).

5. **Result message wording**: Use `"saved plan to <path> (<N> bytes)"` — distinct from `write`'s `"wrote <N> bytes to <path>"` — to make it clear in the chat log which tool was used.

6. **No state/session needed**: Unlike `skill`, this tool does not need `ctx.state` or `ctx.session_id`. It's purely a filesystem operation + pin flag.

---

## Navigation Anchors

- **Primary entry point**: `crates/jinn-domain/src/feat/tools_actor/save_plan.rs` (new file)
- **Registration**: `crates/jinn-domain/src/feat/tools_actor/registry.rs` → `builtin_tools()`
- **Module declaration**: `crates/jinn-domain/src/feat/tools_actor/mod.rs`
- **Reference tool (structure)**: `crates/jinn-domain/src/feat/tools_actor/write.rs`
- **Reference tool (pinning)**: `crates/jinn-domain/src/feat/tools_actor/skill.rs`

---

## Dependency Mappings

No new external dependencies. The tool uses the same crates as `write`:
- `tokio` (async filesystem operations)
- `serde_json` (argument parsing)
- `jinn_provider` (`ToolResult`, `ToolResultPinPosition`, `ToolDefinition`, `ToolCall` types, re-exported via `tool_types`)

---

## Test Strategies

All tests go in `#[cfg(test)] mod tests` inside `save_plan.rs`. Follow the existing BDD pattern from `write.rs`.

### `test_ctx()` helper
Identical to `write::tests::test_ctx()` — a `ToolContext` with `cwd: PathBuf::from("/tmp")` and all optional fields set to `None`.

### Test cases

| # | Test Name | Given | When | Then |
|---|-----------|-------|------|------|
| 1 | `definition_has_correct_name` | The definition | Inspected | `name == "save_plan"` |
| 2 | `definition_requires_path_and_content` | The definition parameters | Inspected | `required` array contains both `"path"` and `"content"` |
| 3 | `definition_mentions_plans_dir_in_guidelines` | The definition guidelines | Inspected | At least one guideline string contains `.plans` |
| 4 | `execute_writes_file_content` | A temp dir + valid call | Execute | File on disk contains the written content |
| 5 | `execute_pins_on_success` | A temp dir + valid call | Execute | `result.pin_position == Some(Relative)` |
| 6 | `execute_no_pin_on_bad_json` | Invalid JSON arguments | Execute | `result.pin_position == None` and `success == false` |
| 7 | `execute_no_pin_on_write_error` | A path that cannot be written (e.g. permissions) | Execute | `result.pin_position == None` and `success == false` |
| 8 | `execute_returns_path_not_content` | A temp dir + valid call with known content | Execute | `result.content` contains the file path and byte count but NOT the plan body text |
| 9 | `execute_creates_parent_dirs` | A deeply nested path like `.plans/task/plan.md` | Execute | File is created, parent directories exist |
| 10 | `execute_resolves_relative_path` | A ToolContext with a temp CWD + relative path | Execute | File is created at `cwd/path` |
| 11 | `execute_returns_error_on_bad_json` | `"not json"` as arguments | Execute | `success == false`, content mentions parse failure |

---

## Acceptance Criteria

1. The `save_plan` tool appears in the LLM's tool list with the correct name, description, and `.plans/` guidelines.
2. Calling `save_plan` with a `path` and `content` writes the file to disk identically to `write`.
3. On success, the `ToolResult` has `pin_position: Some(Relative)`.
4. On failure (bad JSON, write error, directory creation failure), the `ToolResult` has `pin_position: None` and `success: false`.
5. The result content contains the file path and byte count, not the plan body.
6. Relative paths resolve against the session CWD.
7. Parent directories are created automatically.
8. All existing tests continue to pass.

---

## Phases

### Phase 1: Implement `save_plan.rs`
Create the new tool module with `definition()`, `execute()`, and `parse_args()` functions. The implementation mirrors `write.rs` with pinning on success and plan-specific wording.

### Phase 2: Register and Wire Up
Add the module declaration to `mod.rs` and the tool entry to `registry.rs::builtin_tools()`.

### Phase 3: Tests
Write the BDD-style test suite inside `save_plan.rs` covering all test cases listed above.

### Phase 4: Verification
Run `just check` and `just test` to confirm everything compiles and all tests pass (new and existing).
