# Phase 1: `nsslice-chat-input-box-protocol` — create `state/` module, fix foreign impl

## Problem

`autocomplete.rs` defines `AutocompleteState` and `AutocompleteMatch` but also contains `impl ChatInputBoxState` (143 lines). The struct definition for `ChatInputBoxState` lives in `lib.rs`. This violates the rule: struct definition and all its impls should live together.

## What Moves / What Stays

### Moves
- `ChatInputBoxState` struct + ALL its impl blocks (from both `lib.rs` and `autocomplete.rs`) → `state/chat_input_box.rs`
- `AutocompleteState` struct + its `impl` block (from `autocomplete.rs`) → `state/autocomplete.rs`
- `AutocompleteMatch` struct (from `autocomplete.rs`) → `lib.rs`

### Stays
- Everything else in `lib.rs` (module doc, imports, test module)
- All test code (stays in `tests/` via `mod tests`)

### Deleted
- `autocomplete.rs`

## File Changes

1. **Create `state/chat_input_box.rs`** — `ChatInputBoxState` struct + both `impl` blocks + `Default` impl
2. **Create `state/autocomplete.rs`** — `AutocompleteState` struct + `impl AutocompleteState` only
3. **Create `state.rs`** — `mod chat_input_box; mod autocomplete;` + re-exports
4. **Modify `lib.rs`** — replace `mod autocomplete;` with `mod state;`, add `AutocompleteMatch` struct definition, remove `ChatInputBoxState` and its impls
5. **Delete `autocomplete.rs`**

## Acceptance Criteria

- [ ] `state/chat_input_box.rs` exists and contains `ChatInputBoxState` + all its impls
- [ ] `state/autocomplete.rs` exists and contains `AutocompleteState` + its impl only
- [ ] `state.rs` exists with re-exports
- [ ] `lib.rs` contains `AutocompleteMatch` and no `impl ChatInputBoxState`
- [ ] `autocomplete.rs` is deleted
- [ ] `just check` passes
- [ ] `just test` passes
- [ ] `just lint` passes
