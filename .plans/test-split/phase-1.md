# Phase 1: Clean up all existing `#[path]` attributes

## Problem

16 files in `crates/` use `#[path = "..."]` attributes on `mod` declarations for test modules. These attributes are unnecessary — Rust resolves `mod foo_tests;` to `foo_tests.rs` by default. Three files have `mod` names that don't match their filenames (which is why `#[path]` was added as a workaround). One file has a dangling `#[path]` with no `mod` following it.

## What Changes / What Stays

**Changes:**
- 15 files: remove the `#[path = "..."]` line, keep `#[cfg(test)]` and `mod <name>;`
- 3 of those 15 also rename `mod <name>;` to match the filename
- 1 file: remove the dangling `#[path = "state_tests.rs"]` + its orphaned `#[cfg(test)]`

**Stays:**
- All `_tests.rs` files remain unchanged (no renames needed)
- All test code remains unchanged

## File Changes

### Group A: Remove `#[path]` only (mod name already matches filename) — 12 files

Each file has this pattern at the bottom:
```rust
#[cfg(test)]
#[path = "<name>_tests.rs"]
mod <name>_tests;
```
Becomes:
```rust
#[cfg(test)]
mod <name>_tests;
```

1. `crates/slices/nsslice-chat-input-box/src/autocomplete_render.rs` — `mod autocomplete_render_tests;`
2. `crates/slices/nsslice-provider/src/render.rs` — `mod render_tests;`
3. `crates/slices/nsslice-provider/src/entries.rs` — `mod entries_tests;`
4. `crates/common/nullslop-actor/src/context.rs` — `mod context_tests;`
5. `crates/common/nullslop-component/src/chat_session/state.rs` — `mod state_tests;`
6. `crates/common/nullslop-workflow/src/builder.rs` — `mod builder_tests;`
7. `crates/common/nullslop-workflow/src/state.rs` — `mod state_tests;`
8. `crates/common/nullslop-workflow/src/guard.rs` — `mod guard_tests;`
9. `crates/common/nullslop-tui/src/render.rs` — `mod render_tests;`
10. `crates/common/nullslop-selection-widget/src/state.rs` — `mod state_tests;`
11. `crates/common/nullslop-protocol/src/chat.rs` — `mod chat_tests;`
12. `crates/common/nullslop-providers/src/registry.rs` — `mod registry_tests;`

### Group B: Remove `#[path]` + rename `mod` to match filename — 3 files

13. `crates/slices/nsslice-picker/src/strategy_entries.rs`:
    ```rust
    #[cfg(test)]
    #[path = "strategy_entries_tests.rs"]
    mod entries_tests;
    ```
    →
    ```rust
    #[cfg(test)]
    mod strategy_entries_tests;
    ```

14. `crates/slices/nsslice-picker/src/render.rs`:
    ```rust
    #[cfg(test)]
    #[path = "picker_render_tests.rs"]
    mod render_tests;
    ```
    →
    ```rust
    #[cfg(test)]
    mod picker_render_tests;
    ```

15. `crates/common/nullslop-intent/src/handler.rs`:
    ```rust
    #[cfg(test)]
    #[path = "handler_tests.rs"]
    mod tests;
    ```
    →
    ```rust
    #[cfg(test)]
    mod handler_tests;
    ```

### Group C: Remove dangling `#[path]` — 1 file

16. `crates/slices/nsslice-chat-input-box-protocol/src/lib.rs` (lines 586–588):
    ```rust
    #[cfg(test)]
    #[path = "state_tests.rs"]

    #[cfg(test)]
    ```
    Remove the first `#[cfg(test)]`, the `#[path = "state_tests.rs"]`, and the blank line. The second `#[cfg(test)]` (which starts the actual inline `mod tests`) stays.

## Implementation Order

1. Apply all Group A edits (12 files, same mechanical change)
2. Apply all Group B edits (3 files, remove `#[path]` + rename `mod`)
3. Apply Group C edit (1 file, remove dangling lines)
4. Run `just check`

## Acceptance Criteria

- [x] Zero `#[path =` attributes remain in `crates/` directory (`grep -r '#\[path =' --include="*.rs" crates/` returns empty — exit code 1)
- [x] `nsslice-picker/src/strategy_entries.rs` has `mod strategy_entries_tests;` (not `entries_tests`)
- [x] `nsslice-picker/src/render.rs` has `mod picker_render_tests;` (not `render_tests`)
- [x] `nullslop-intent/src/handler.rs` has `mod handler_tests;` (not `tests`)
- [x] Dangling `#[path = "state_tests.rs"]` removed from `nsslice-chat-input-box-protocol/src/lib.rs`
- [x] `just check` passes
