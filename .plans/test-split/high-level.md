# test-split

## Problem

28 source files have inline `#[cfg(test)] mod tests` blocks exceeding 200 lines, bloating the source files and making it harder to focus on production code. Additionally, 16 existing test extractions use unnecessary `#[path = "..."]` attributes — the `#[path]` attribute was never needed in this codebase since Rust resolves `mod foo;` by looking for `foo.rs` in the same directory. Three of these have `mod` names that don't match their filenames, which is why `#[path]` was used as a band-aid. One file (`nsslice-chat-input-box-protocol/src/lib.rs`) has a dangling `#[path = "state_tests.rs"]` with no `mod` declaration following it.

The goal is to extract all oversized inline test suites into separate `_tests.rs` files and remove every `#[path]` attribute in `crates/`.

## Key Decisions

- **No `#[path]` attributes.** The `#[path]` attribute is unnecessary and should never have been in this codebase. Every `mod foo_tests;` declaration resolves to `foo_tests.rs` by default — that's how Rust works.
- **Module naming matches the source file being tested.** If the source is `strategy_entries.rs`, the test module is `mod strategy_entries_tests;` and the file is `strategy_entries_tests.rs`.
- **Tests that access private fields stay inline.** The user indicated there shouldn't be many of these. If a specific test accesses private state, it remains in the source file's `#[cfg(test)]` block.
- **License headers** are included in all new `_tests.rs` files, matching the existing pattern (see `handler_tests.rs` for reference).
- **`use super::*;`** is used in extracted test files to access the parent module's public API, matching the established pattern.

## Extraction Pattern

For each file, the inline block at the bottom:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    // ... hundreds of lines of tests ...
}
```

Becomes:

```rust
#[cfg(test)]
mod <source_name>_tests;
```

And a new `<source_name>_tests.rs` file is created alongside the source with the test contents. The `mod` name must match the filename exactly (no `#[path]` needed).

For example, `actor.rs` gets `mod actor_tests;` → `actor_tests.rs`, `lib.rs` gets `mod lib_tests;` → `lib_tests.rs`, `element.rs` gets `mod element_tests;` → `element_tests.rs`.

## Phases

- [ ] **Phase 1: Clean up all existing `#[path]` attributes (16 files)**
  - [ ] Remove `#[path = "..."]` lines from all 13 files where mod name already matches filename:
    - `crates/slices/nsslice-picker/src/render.rs` — but mod name is `render_tests` while file is `picker_render_tests.rs` (see mismatch fix below)
    - `crates/slices/nsslice-chat-input-box/src/autocomplete_render.rs` — `mod autocomplete_render_tests;` → file matches
    - `crates/slices/nsslice-provider/src/render.rs` — `mod render_tests;` → file matches
    - `crates/slices/nsslice-provider/src/entries.rs` — `mod entries_tests;` → file matches
    - `crates/common/nullslop-actor/src/context.rs` — `mod context_tests;` → file matches
    - `crates/common/nullslop-component/src/chat_session/state.rs` — `mod state_tests;` → file matches
    - `crates/common/nullslop-workflow/src/builder.rs` — `mod builder_tests;` → file matches
    - `crates/common/nullslop-workflow/src/state.rs` — `mod state_tests;` → file matches
    - `crates/common/nullslop-workflow/src/guard.rs` — `mod guard_tests;` → file matches
    - `crates/common/nullslop-tui/src/render.rs` — `mod render_tests;` → file matches
    - `crates/common/nullslop-selection-widget/src/state.rs` — `mod state_tests;` → file matches
    - `crates/common/nullslop-protocol/src/chat.rs` — `mod chat_tests;` → file matches
    - `crates/common/nullslop-providers/src/registry.rs` — `mod registry_tests;` → file matches
  - [ ] Fix 3 mod name mismatches (rename `mod` to match filename):
    - `crates/slices/nsslice-picker/src/strategy_entries.rs`: `mod entries_tests;` → `mod strategy_entries_tests;` (file is `strategy_entries_tests.rs`)
    - `crates/slices/nsslice-picker/src/render.rs`: `mod render_tests;` → `mod picker_render_tests;` (file is `picker_render_tests.rs`)
    - `crates/common/nullslop-intent/src/handler.rs`: `mod tests;` → `mod handler_tests;` (file is `handler_tests.rs`)
  - [ ] Remove dangling `#[path = "state_tests.rs"]` in `crates/slices/nsslice-chat-input-box-protocol/src/lib.rs` (line 586–587, it has no `mod` following it)
  - [ ] Run `just check`

- [ ] **Phase 2: Extract the 4 largest test suites (800+ lines)**
  - [ ] `crates/slices/nsslice-context/src/actor.rs` (1020 test lines) → `actor_tests.rs`
  - [ ] `crates/slices/nsslice-session-management/src/actor.rs` (1013 test lines) → `actor_tests.rs`
  - [ ] `crates/slices/nsslice-tools/src/lib.rs` (873 test lines) → `lib_tests.rs`
  - [ ] `crates/slices/nsslice-llm/src/lib.rs` (767 test lines) → `lib_tests.rs`
  - [ ] Run `just check`

- [ ] **Phase 3: Extract next 5 (500–800 lines)**
  - [ ] `crates/common/nullslop-tui/src/app.rs` (586 test lines) → `app_tests.rs`
  - [ ] `crates/slices/nsslice-chat-log/src/element.rs` (553 test lines) → `element_tests.rs`
  - [ ] `crates/slices/nsslice-picker/src/intent.rs` (467 test lines) → `intent_tests.rs`
  - [ ] `crates/slices/nsslice-chat-input-box-protocol/src/lib.rs` (453 test lines) → `lib_tests.rs`
  - [ ] `crates/common/nullslop-actor-host/src/in_memory.rs` (446 test lines) → `in_memory_tests.rs`
  - [ ] Run `just check`

- [ ] **Phase 4: Extract next 5 (400–500 lines)**
  - [ ] `crates/common/nullslop-selection-widget/src/widget.rs` (440 test lines) → `widget_tests.rs`
  - [ ] `crates/common/nullslop-providers/src/fake.rs` (432 test lines) → `fake_tests.rs`
  - [ ] `crates/slices/nsslice-session-management-protocol/src/session_store.rs` (388 test lines) → `session_store_tests.rs`
  - [ ] `crates/common/nullslop-tui/src/split_borders.rs` (360 test lines) → `split_borders_tests.rs`
  - [ ] `crates/slices/nsslice-chat-input-box/src/intent.rs` (348 test lines) → `intent_tests.rs`
  - [ ] Run `just check`

- [ ] **Phase 5: Extract next 5 (300–400 lines)**
  - [ ] `crates/slices/nsslice-dashboard/src/element.rs` (345 test lines) → `element_tests.rs`
  - [ ] `crates/common/nullslop-protocol/src/provider/convert.rs` (338 test lines) → `convert_tests.rs`
  - [ ] `crates/slices/nsslice-context-protocol/src/strategy/token_budget.rs` (329 test lines) → `token_budget_tests.rs`
  - [ ] `crates/slices/nsslice-chat-input-box/src/element.rs` (329 test lines) → `element_tests.rs`
  - [ ] `crates/slices/nsslice-status-bar/src/element.rs` (305 test lines) → `element_tests.rs`
  - [ ] Run `just check`

- [ ] **Phase 6: Extract remaining 9 (<300 lines)**
  - [ ] `crates/common/nullslop-tui/src/selection.rs` (271 test lines) → `selection_tests.rs`
  - [ ] `crates/slices/nsslice-context-protocol/src/strategy/compaction.rs` (264 test lines) → `compaction_tests.rs`
  - [ ] `crates/common/nullslop-providers/src/registry_service.rs` (254 test lines) → `registry_service_tests.rs`
  - [ ] `crates/common/nullslop-protocol/src/key.rs` (253 test lines) → `key_tests.rs`
  - [ ] `crates/common/nullslop-providers/src/convert.rs` (242 test lines) → `convert_tests.rs`
  - [ ] `crates/slices/nsslice-context-protocol/src/strategy/sliding_window.rs` (239 test lines) → `sliding_window_tests.rs`
  - [ ] `crates/slices/nsslice-pinned-panel/src/intent.rs` (236 test lines) → `intent_tests.rs`
  - [ ] `crates/common/nullslop-prompt-template/src/store.rs` (214 test lines) → `store_tests.rs`
  - [ ] `crates/common/nullslop-actor/src/actor_ref.rs` (211 test lines) → `actor_ref_tests.rs`
  - [ ] Run `just check`

## Acceptance Criteria

- All 28 files with 200+ lines of inline tests have their test code extracted to `_tests.rs` sibling files
- Zero `#[path]` attributes remain in `crates/`
- The 3 existing mod name mismatches are corrected
- The dangling `#[path]` in `nsslice-chat-input-box-protocol/src/lib.rs` is removed
- `just test` passes — no tests broken
- `just lint` passes — no warnings introduced
