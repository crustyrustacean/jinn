# Compaction Worker Fix: Strip Tool Payloads from Summarization Input

## Problem

The compaction serializer (`serialize_entries_for_compaction()`) sends full tool call arguments and tool result content (up to 2KB each) to the summarization LLM. Despite the `_compaction.md` prompt instructing the model not to echo code/output, the sheer volume of tool data dominates the serialized text, causing the model to produce summaries that are essentially reformatted tool output rather than narrative summaries.

## Solution

Change `serialize_entries_for_compaction()` in `serializer.rs` to:
- Emit `[Tool call]: <name>` (just the tool name, no arguments) for `ToolCall` entries
- Skip `ToolResult` entries entirely (produce no output line)

No changes to the algorithm, worker, or token accounting — tool entries still count toward the compaction boundary calculation, they just don't carry their payload to the summarization LLM.

## Acceptance Criteria

- `serialize_entries_for_compaction()` emits `[Tool call]: <name>` (no arguments) for tool calls
- `serialize_entries_for_compaction()` emits nothing for tool results
- Tool entries are still counted by `gather_compactable_entries()` for token budget (algorithm.rs unchanged)
- All existing tests pass (with updated assertions for the new serialization format)
- No changes to `worker.rs`, `algorithm.rs`, or token estimation logic

---

## Dialectical Outcomes (Why)

### Decision: Skip tool results entirely, emit tool-call names only

**Alternatives considered:**
- **Skip both ToolCall and ToolResult entirely (Option A):** Rejected because long conversations where the assistant does 20 tool calls would appear as if the assistant said nothing. The `Assistant` entries between tool calls usually describe intent, but keeping `[Tool call]: <name>` preserves the signal "the assistant ran bash and read" without the payload.
- **Collapse consecutive tool calls into `[Ran tools: bash, read]`:** Deferred. Start with one line per call; if summaries don't improve, collapse later. Collapsing introduces grouping complexity (tracking consecutive tool calls vs interleaved assistant messages) for uncertain benefit.

**Trade-off:** We lose the specific arguments and results, but the compaction prompt (`_compaction.md`) already instructs the model to produce a narrative summary. The assistant's own `Assistant` entries typically describe what it's doing ("Let me check the files", "I found the issue in main.rs"), so the narrative thread is preserved without tool payloads.

### Decision: Keep token accounting unchanged

Tool entries still consume real context window budget even though we don't send their content to the summarizer. The compaction boundary algorithm must still account for them to correctly decide which entries to compact.

---

## Relevant Files (Where)

| File | Action | Purpose |
|------|--------|---------|
| `crates/jinn-domain/src/feat/compaction_worker/serializer.rs` | **Modify** | Core change: strip tool payloads from serialization |
| `crates/jinn-domain/src/feat/compaction_worker/serializer.rs` (tests module) | **Modify** | Update test assertions to match new format |

No other files need modification.

---

## Key Code Context (What)

### Current `ToolCall` match arm (lines 53–57 of serializer.rs) — must be changed:

```rust
ChatEntryKind::ToolCall {
    name, arguments, ..
} => {
    lines.push(format!("[Tool call]: {name}({arguments})"));
}
```

### Current `ToolResult` match arm (lines 58–74 of serializer.rs) — must be changed:

```rust
ChatEntryKind::ToolResult { name, content, .. } => {
    let truncated = if content.len() > TOOL_RESULT_MAX_BYTES {
        let safe_end = grapheme_safe_end(content, TOOL_RESULT_MAX_BYTES);
        let candidate = &content[..safe_end];
        let mut end = safe_end;
        // Try to break at a newline or space.
        if let Some(pos) = candidate.rfind('\n') {
            end = pos;
        } else if let Some(pos) = candidate.rfind(' ') {
            end = pos;
        }
        format!("{}... (truncated)", &content[..end])
    } else {
        content.clone()
    };
    lines.push(format!("[Tool result] {name}: {truncated}"));
}
```

### Current skip arm (lines 78–84) — `ToolResult` and `ToolCall` are NOT in this list:

```rust
ChatEntryKind::System(_)
| ChatEntryKind::Error(_)
| ChatEntryKind::Thinking(_)
| ChatEntryKind::Transient(_)
| ChatEntryKind::Skill { .. }
| ChatEntryKind::Compaction { .. } => {}
```

### `ChatEntryKind` enum definition (for reference, in `chat_entry.rs`):

The relevant variants are:
```rust
ChatEntryKind::ToolCall { id, name, arguments }
ChatEntryKind::ToolResult { id, name, content, status }
```

### Existing tests that need updating (serializer.rs test module):

1. **`serializes_tool_call_entry`** (line 112) — currently asserts:
   ```rust
   assert_eq!(result, r#"[Tool call]: bash({"command":"ls"})"#);
   ```

2. **`serializes_tool_result_entry`** (line 118) — currently asserts tool result IS present:
   ```rust
   assert_eq!(result, "[Tool result] bash: file1.txt\nfile2.txt");
   ```

3. **`truncates_long_tool_result`** (line 130) — asserts tool result truncation works.

4. **`truncates_tool_result_with_multibyte_at_boundary`** (line 182) — asserts truncation.

5. **`truncates_tool_result_with_emoji`** (line 204) — asserts truncation.

6. **`serialize_tool_result_at_exact_boundary`** (line 322) — asserts no truncation at exact 2000 bytes.

7. **`serializes_mixed_entries`** (line 158) — asserts 5 lines including `[Tool call]` and `[Tool result]`:
   ```rust
   assert_eq!(lines.len(), 5);
   assert!(lines[2].starts_with("[Tool call]"));
   assert!(lines[3].starts_with("[Tool result]"));
   ```

### Constants that become dead code:

```rust
const TOOL_RESULT_MAX_BYTES: usize = 2000;
```

And the helper function:
```rust
fn grapheme_safe_end(text: &str, max_bytes: usize) -> usize { ... }
```

These are only used in the `ToolResult` serialization path and the grapheme-specific tests. After the change, the constant and function should be removed, along with the `unicode_segmentation` dependency import (line 7) and the `grapheme_safe_end` tests.

---

## Implementation Algorithm (How)

### Phase 1: Update serializer

In `serialize_entries_for_compaction()` in `serializer.rs`:

1. **Modify the `ToolCall` arm** to emit only the name:
   ```rust
   ChatEntryKind::ToolCall { name, .. } => {
       lines.push(format!("[Tool call]: {name}"));
   }
   ```

2. **Move `ToolResult` into the skip arm** so it produces no output:
   ```rust
   ChatEntryKind::System(_)
   | ChatEntryKind::Error(_)
   | ChatEntryKind::Thinking(_)
   | ChatEntryKind::Transient(_)
   | ChatEntryKind::Skill { .. }
   | ChatEntryKind::Compaction { .. }
   | ChatEntryKind::ToolResult { .. } => {}
   ```

3. **Remove dead code:**
   - Remove `const TOOL_RESULT_MAX_BYTES: usize = 2000;`
   - Remove `fn grapheme_safe_end(...)`
   - Remove `use unicode_segmentation::UnicodeSegmentation;`

### Phase 2: Update serializer tests

1. **`serializes_tool_call_entry`** — update assertion:
   ```rust
   assert_eq!(result, "[Tool call]: bash");
   ```

2. **`serializes_tool_result_entry`** — change to assert tool result produces NO output:
   ```rust
   let result = serialize_entries_for_compaction(&entries);
   assert!(result.is_empty());
   ```

3. **`truncates_long_tool_result`** — change to assert empty output.

4. **`truncates_tool_result_with_multibyte_at_boundary`** — change to assert empty output (or remove entirely since truncation logic is gone).

5. **`truncates_tool_result_with_emoji`** — change to assert empty output (or remove).

6. **`serialize_tool_result_at_exact_boundary`** — remove (no truncation boundary to test).

7. **`serializes_mixed_entries`** — update: expect 4 lines instead of 5, no `[Tool result]` line, `[Tool call]` has no arguments:
   ```rust
   assert_eq!(lines.len(), 4);
   assert!(lines[0].starts_with("[User]"));
   assert!(lines[1].starts_with("[Assistant]"));
   assert!(lines[2].starts_with("[Tool call]"));
   assert_eq!(lines[2], "[Tool call]: bash");
   assert!(lines[3].starts_with("[Assistant]"));
   ```

8. **Remove all `grapheme_safe_end` tests** (lines 224–319): `grapheme_safe_end_returns_len_for_short_string`, `grapheme_safe_end_returns_len_for_exact_match`, `grapheme_safe_end_truncates_at_grapheme_boundary`, `grapheme_safe_end_stops_before_boundary`, `grapheme_safe_end_zero_max_bytes`, `grapheme_safe_end_empty_string`, `grapheme_safe_end_single_multibyte_grapheme`, `grapheme_safe_end_single_multibyte_grapheme_fits`.

9. **Add new test** `serializes_tool_call_name_only` verifying tool call arguments are absent:
   ```rust
   #[test]
   fn serializes_tool_call_name_only() {
       let entries = vec![ChatEntry::tool_call("id1", "bash", r#"{"command":"ls -la && cat secret.txt"}"#)];
       let result = serialize_entries_for_compaction(&entries);
       assert_eq!(result, "[Tool call]: bash");
   }
   ```

10. **Add new test** `skips_tool_result_entry` verifying tool results produce no output:
    ```rust
    #[test]
    fn skips_tool_result_entry() {
        let entries = vec![ChatEntry::tool_result(
            "id1",
            "bash",
            "sensitive output that should never reach the LLM",
            ToolResultStatus::Success,
        )];
        let result = serialize_entries_for_compaction(&entries);
        assert!(result.is_empty());
    }
    ```

### Phase 3: Run full test suite

```bash
cargo test --package jinn-domain compaction_worker
cargo test --package jinn-domain compaction
cargo test --package jinn-domain serializer
```

---

## Anti-Goals (Out of Scope)

- **Collapsing consecutive tool calls** into `[Ran tools: bash, read]` — deferred to a follow-up if summaries don't improve.
- **Changing the compaction algorithm** (`algorithm.rs`) — token counting and boundary logic stay the same.
- **Changing the compaction worker** (`worker.rs`) — the LLM call and mutation production stay the same.
- **Changing the compaction prompt** (`res/prompts/_compaction.md`) — the prompt already says not to echo code/output.
- **Changing token estimation** for tool entries — they still consume real context window budget.
- **Filtering tool entries from `gather_compactable_entries()`** — tool entries must still be gathered and excluded from context via `ForcedExclude`.

---

## Edge Cases & Gotchas

1. **`ToolResult` must be in the skip arm, not just omitted from the match.** Rust requires exhaustive matching on enums. Adding `ChatEntryKind::ToolResult { .. }` to the existing skip arm is the correct approach.

2. **The `..` pattern in `ToolCall`.** The `ToolCall` variant has fields `{ id, name, arguments }`. Using `ChatEntryKind::ToolCall { name, .. }` captures only `name` and ignores `id` and `arguments`. This is correct and idiomatic.

3. **Dead code warning for `unicode_segmentation` import.** After removing `grapheme_safe_end`, the `use unicode_segmentation::UnicodeSegmentation;` import becomes unused. Remove it to avoid compiler warnings. Check `Cargo.toml` to see if `unicode-segmentation` is used elsewhere in this crate before considering removing the dependency.

4. **Worker integration tests are unaffected.** The tests in `worker_tests.rs` use `FakeLlmServiceFactory` that returns a canned summary regardless of input. They test the worker pipeline, not the serialization format. No changes needed there.

5. **The `Actor` variant is still serialized.** `ChatEntryKind::Actor { source, text }` produces `[Actor: {source}]: {text}` — this is unchanged and correct.

---

## Navigation Anchors

- **Primary entry point:** `serialize_entries_for_compaction()` in `crates/jinn-domain/src/feat/compaction_worker/serializer.rs` (line 42)
- **Match arms to modify:** Lines 46–85 (the main match on `entry.kind`)
- **Tests to update:** Lines 92–338 (the `#[cfg(test)] mod tests` block)

---

## Dependency Mappings

### Removals
- `unicode-segmentation` import in `serializer.rs` — becomes dead code after removing `grapheme_safe_end`. Check if other files in this crate use it before removing from `Cargo.toml`.

### No new dependencies required.

---

## Phases

- [ ] **Phase 1: Update serializer** — Modify `ToolCall` and `ToolResult` match arms in `serialize_entries_for_compaction()`. Remove dead code (`TOOL_RESULT_MAX_BYTES`, `grapheme_safe_end`, `unicode_segmentation` import).
- [ ] **Phase 2: Update serializer tests** — Fix existing test assertions for new format. Remove `grapheme_safe_end` tests. Add `serializes_tool_call_name_only` and `skips_tool_result_entry` tests.
- [ ] **Phase 3: Run full test suite** — `cargo test --package jinn-domain compaction_worker` and verify no regressions across worker tests, algorithm tests, and serializer tests.
