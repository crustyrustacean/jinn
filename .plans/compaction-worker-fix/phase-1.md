# Phase 1: Update serializer — strip tool payloads

## Problem
The compaction serializer sends full tool call arguments and tool result content to the summarization LLM, causing the model to produce summaries that echo code/output instead of narrative summaries.

## What Moves / What Stays
- **Moves:** `ToolCall` arm emits only name (no arguments). `ToolResult` arm moves to skip group.
- **Stays:** All other match arms, function signature, algorithm.rs, worker.rs unchanged.

## File Changes

### 1. `crates/jinn-domain/src/feat/compaction_worker/serializer.rs` — MODIFIED

**Change A:** Remove `unicode_segmentation` import (line 7):
```
-use unicode_segmentation::UnicodeSegmentation;
```

**Change B:** Remove `TOOL_RESULT_MAX_BYTES` constant (lines 11–12):
```
-const TOOL_RESULT_MAX_BYTES: usize = 2000;
```

**Change C:** Remove `grapheme_safe_end` function (lines 14–30).

**Change D:** Update doc comment (lines 32–41) to reflect new format.

**Change E:** Modify `ToolCall` arm (lines 53–57) — emit only name:
```rust
ChatEntryKind::ToolCall { name, .. } => {
    lines.push(format!("[Tool call]: {name}"));
}
```

**Change F:** Move `ToolResult` into skip arm (lines 58–84) — remove the entire ToolResult match body and add `ToolResult` to the skip list:
```rust
ChatEntryKind::System(_)
| ChatEntryKind::Error(_)
| ChatEntryKind::Thinking(_)
| ChatEntryKind::Transient(_)
| ChatEntryKind::Skill { .. }
| ChatEntryKind::Compaction { .. }
| ChatEntryKind::ToolResult { .. } => {}
```

## Implementation Order
1. Apply all serializer.rs production code changes (A–F) in a single edit.
2. Build to verify no compilation errors.

## Acceptance Criteria
- [ ] `serialize_entries_for_compaction()` emits `[Tool call]: <name>` (no arguments)
- [ ] `serialize_entries_for_compaction()` emits nothing for `ToolResult` entries
- [ ] `TOOL_RESULT_MAX_BYTES`, `grapheme_safe_end`, and `unicode_segmentation` import removed
- [ ] File compiles without errors or warnings
