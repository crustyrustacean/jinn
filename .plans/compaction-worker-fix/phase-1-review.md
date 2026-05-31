# Phase 1: Update serializer — strip tool payloads

## Execution Plan

See phase-1.md for the original plan.

## Changes

- **Modified `serializer.rs`**: Changed `ToolCall` arm to emit only `[Tool call]: {name}` (no arguments). Moved `ToolResult` into the skip arm alongside `System`, `Error`, `Thinking`, etc.
- **Removed dead code**: `TOOL_RESULT_MAX_BYTES` constant, `grapheme_safe_end()` function, and `unicode_segmentation` import (dependency still used elsewhere in the crate).
- **Updated all tests**: Fixed `serializes_tool_call_entry` assertion, changed `serializes_tool_result_entry`, `truncates_long_tool_result`, `truncates_tool_result_with_multibyte_at_boundary`, `truncates_tool_result_with_emoji` to assert empty output. Updated `serializes_mixed_entries` for 4 lines (no tool result line). Removed all `grapheme_safe_end` tests and `serialize_tool_result_at_exact_boundary` test. Added `serializes_tool_call_name_only` and `skips_tool_result_with_content` tests.
- **Updated module doc comment** to reflect new behavior.

## Divergence

None. Implementation matches the spec exactly.

## Verification

All 60 compaction_worker tests pass:
- 9 algorithm tests (unchanged, still pass)
- 11 serializer tests (7 existing updated + 2 new, all pass)
- 40 worker tests (unchanged, still pass)

No clippy warnings in changed files.

## Risks

None. The change is isolated to `serialize_entries_for_compaction()` which is only called from `CompactionWorker::evaluate_with_config()`. The algorithm and worker logic are completely untouched.
