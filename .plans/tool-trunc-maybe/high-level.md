# Tool Result Truncation

## Problem

Tool results (especially `bash` output and `read` file contents) are sent to the LLM in full, consuming large portions of the context window. The `bash` tool can produce thousands of lines of output, and `read` can load entire files — all of it goes verbatim into `ToolResult.content` and then into the LLM prompt via `entries_to_messages`. There is no truncation at any layer.

## Proposed Solution

Implement tool-level truncation inspired by pi-mono's approach, with an important architectural difference: **the entry stores the full content, and only the truncated version is sent to the LLM**. This means:

- Tools compute truncation and produce a `ToolResult` with both `content` (truncated) and `full_content` (original)
- `entries_to_messages` already sends `content` — no changes needed there
- Full output survives in session persistence for UI rendering and future features
- No temp files needed

The truncation layer is in the tools actor. The context assembly layer (`entries_to_messages`) doesn't need changes because `content` will already contain the truncated version. The `full_content` field is purely for persistence and UI.

Truncation limits default to 2000 lines / 50KB (whichever is hit first) and are configurable via preferences.

### Key design decisions (from conversation)

- **Option B chosen**: Store full output in `ChatEntryKind::ToolResult`, truncate only for LLM. No temp files.
- **Context assembly is the right spot** to alter data — confirmed by user. But since `entries_to_messages` already uses `content`, the truncation happens at tool execution time (tools produce truncated `content` + `full_content`).
- **Compaction-time truncation is out of scope** — compaction is a separate future feature.
- **bash and read only** — other tools (echo, get_time, skill, write, edit) produce small outputs.

### Truncation strategy per tool

- **bash**: `truncate_tail` — keeps the end of output (errors and final results are at the tail, like pi-mono does)
- **read**: `truncate_head` — keeps the beginning of output (file headers/imports are at the top, like pi-mono does)
- Each tool appends an actionable notice telling the LLM how to get more (e.g., `[Showing lines 1-200 of 800. Use offset=201 to continue.]`)

### pi-mono reference

Explored `/mnt/zed/repos/third-party/pi-mono/packages/coding-agent/src/core/tools/truncate.ts` which provides:
- `truncateHead(content, { maxLines, maxBytes })` → `TruncationResult` with `content`, `truncated`, `truncatedBy`, `totalLines`, `totalBytes`, `outputLines`, `outputBytes`, `firstLineExceedsLimit`
- `truncateTail(content, { maxLines, maxBytes })` → same shape, with `lastLinePartial` edge case
- `truncateLine(line, maxChars)` — for individual long lines (grep)
- Defaults: `DEFAULT_MAX_LINES = 2000`, `DEFAULT_MAX_BYTES = 50KB`

### Config integration

- Add `max_tool_output_lines` and `max_tool_output_bytes` to the preferences/config system
- Thread through `ToolContext` so tools read configured limits
- Default to 2000 lines, 50KB

---

## Phases

- [x] Phase 1: Truncation primitives
  - [x] Create `truncation.rs` in `tools_actor/` with pure truncation functions and a `TruncationResult` struct
  - [x] `truncate_head(content, max_lines, max_bytes)` — keeps the beginning (for `read`)
  - [x] `truncate_tail(content, max_lines, max_bytes)` — keeps the end (for `bash`)
  - [x] Default constants: `DEFAULT_MAX_LINES = 2000`, `DEFAULT_MAX_BYTES = 50KB`
  - [x] Unit tests for all edge cases (first line exceeds byte limit, empty content, content under limits, etc.)

- [x] Phase 2: Update `ChatEntryKind::ToolResult` data model
  - [x] Add `full_content: Option<String>` — stores original untruncated output
  - [x] Add `truncation: Option<TruncationMeta>` — metadata (total lines, total bytes, truncated_by, strategy)
  - [x] `content` becomes the truncated version; `full_content` is `Some(...)` only when truncation occurred
  - [x] Update `Serialize`/`Deserialize` impls
  - [x] Update `text()`, `content_fingerprint()` to handle new fields
  - [x] Fix all match arms on `ChatEntryKind::ToolResult` across the codebase

- [x] Phase 3: Apply truncation in `bash` and `read` tools
  - [x] `builtin_bash.rs`: apply `truncate_tail`, store full output in `full_content`, append actionable notice
  - [x] `builtin_read.rs`: apply `truncate_head`, store full output in `full_content`, append actionable notice with `offset` hint
  - [x] Update tool definitions' description strings to document truncation behavior
  - [x] Thread truncation limits from `ToolContext` (defaulting to constants for now)

- [x] Phase 4: Config integration
  - [x] Add `max_tool_output_lines` and `max_tool_output_bytes` to the config/preferences system
  - [x] Wire settings into `ToolContext` so tools use configured limits instead of hardcoded defaults
  - [x] Default to 2000 lines, 50KB

## Acceptance Criteria

- `bash` output exceeding 2000 lines or 50KB is truncated (tail kept), with a notice appended
- `read` output exceeding 2000 lines or 50KB is truncated (head kept), with a notice and `offset` hint
- Full output is preserved in the `ChatEntryKind::ToolResult` entry
- `entries_to_messages` sends only truncated content to the LLM
- Token estimation uses truncated content (not full) for budgeting
- Truncation thresholds are configurable
- All existing tests pass, new code has tests per BDD style
