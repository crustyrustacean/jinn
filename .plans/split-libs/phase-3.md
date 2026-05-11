# Phase 2: Split `nsslice-chat-input-box-protocol/src/lib.rs`

## Problem

`lib.rs` in `nsslice-chat-input-box-protocol` is 587 lines. It contains the core text buffer logic (`ChatInputBoxState`) plus the autocomplete subsystem (`AutocompleteMatch`, `AutocompleteState`, ~13 autocomplete-related methods). Autocomplete is a self-contained feature bolted onto the input buffer.

## What Moves / What Stays

**Moves to `src/autocomplete.rs`:**
- `AutocompleteMatch` struct
- `AutocompleteState` struct + impl
- All autocomplete-related `impl ChatInputBoxState` methods: `autocomplete()`, `autocomplete_mut()`, `deactivate_autocomplete()`, `activate_autocomplete()`, `update_autocomplete_matches()`, `autocomplete_filter()`, `autocomplete_selected()`, `autocomplete_move_up()`, `autocomplete_move_down()`, `autocomplete_token_start()`, `autocomplete_token_screen_col()`, `complete_autocomplete()`, `expand_autocomplete()`

**Stays in `lib.rs`:**
- `ChatInputBoxState` struct definition
- Core buffer methods: `new()`, `text()`, `is_empty()`, `cursor_pos()`, `grapheme_count()`, `grapheme_at()`, `replace_grapheme_range()`, `insert_text()`, `insert_grapheme_at_cursor()`, `delete_grapheme_before_cursor()`, `delete_grapheme_after_cursor()`, `reset()`, `replace_all()`, `move_cursor_*`, `visual_line_count()`, `cursor_row_col()`, `grapheme_index_for_row_col()`

## Acceptance Criteria

- [x] `crates/slices/nsslice-chat-input-box-protocol/src/autocomplete.rs` exists with autocomplete types + methods
- [x] `lib.rs` no longer contains `AutocompleteMatch`, `AutocompleteState`, or autocomplete-related impl methods
- [x] `just check` passes
- [x] `just test` passes
- [x] `just lint` passes

---

## Review: Phase 3 — Split nsslice-chat-input-box-protocol lib.rs

### Changes

Extracted `AutocompleteMatch`, `AutocompleteState`, and all autocomplete-related methods on `ChatInputBoxState` into `src/autocomplete.rs`. The autocomplete impl block accesses private fields (`input_buffer`, `cursor_pos`, `autocomplete`) via Rust's submodule visibility rules — submodules can access private items of their parent.

### Divergence Summary

None.

### Verification

- `just check` — clean
- `just test` — all pass
- `just lint` — pass

### Risks

None.

### Next Steps

Phase 4: Split `nsslice-session-management/src/actor.rs`.
