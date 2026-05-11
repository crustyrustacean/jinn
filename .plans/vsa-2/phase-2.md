# Phase 2: Extract Picker System → `nsslice-picker`

## Problem

The picker system (9 intents, 9 validators, ~15 handler tests, 8 validator tests) lives entirely in `nullslop-intent`. This phase extracts it into a dedicated `nsslice-picker` slice crate. The picker has no element (no rendering) — it's purely intent handling and validation.

The main complexity is the `confirm_keymap` function, which recursively calls `IntentHandler::handle()`. Moving it to the slice would create a circular dependency. The solution: `handle_picker_confirm` returns `(IntentResult, Option<Intent>)` — for keymap confirm, it returns the selected intent as the second element. The `nullslop-intent` caller re-dispatches.

## What Moves

### Validator: `nullslop-intent/src/validators/picker.rs` → `nsslice-picker/src/validator.rs`

Move the entire file contents:
- 7 infallible validators: `validate_picker_insert_char`, `validate_picker_backspace`, `validate_picker_move_up`, `validate_picker_move_down`, `validate_picker_move_cursor_left`, `validate_picker_move_cursor_right`, `validate_toggle_keymap_scope_filter`
- 2 fallible validators: `validate_picker_confirm` (→ `PickerConfirmError`), `validate_open_picker` (→ `OpenPickerError`)
- 2 error enums: `PickerConfirmError`, `OpenPickerError`
- 8 validator tests (move entire `#[cfg(test)] mod tests` block)

### Handler functions: `nullslop-intent/src/handler.rs` → `nsslice-picker/src/intent.rs`

**Functions to move** (from the `// --- Picker handlers ---` section):

1. `handle_open_picker(state, kind) -> IntentResult`
2. `handle_picker_confirm(state) -> IntentResult` — **modified** to return `(IntentResult, Option<Intent>)` instead of re-dispatching
3. `confirm_provider(state) -> IntentResult`
4. `confirm_strategy(state) -> IntentResult`
5. `confirm_keymap(state) -> (IntentResult, Option<Intent>)` — **changed**: no longer calls `IntentHandler::handle()`, instead returns the intent
6. `confirm_session(state) -> IntentResult`
7. `handle_toggle_keymap_scope_filter(state) -> IntentResult`

**Constant to move**: `PICKER_MAX_VISIBLE` (was `IntentHandler::PICKER_MAX_VISIBLE`) → module-level constant in `intent.rs`.

**Inline match arms to move** (become dedicated handler functions):
- `PickerInsertChar { ch }` → `handle_insert_char(state, ch) -> IntentResult`
- `PickerBackspace` → `handle_backspace(state) -> IntentResult`
- `PickerMoveUp` → `handle_move_up(state) -> IntentResult`
- `PickerMoveDown` → `handle_move_down(state) -> IntentResult`
- `PickerMoveCursorLeft` → `handle_move_cursor_left(state) -> IntentResult`
- `PickerMoveCursorRight` → `handle_move_cursor_right(state) -> IntentResult`

### Tests: `nullslop-intent/src/handler_tests.rs` → `nsslice-picker/src/intent.rs`

**14 tests to move** (from the `// ============ Picker Intents ============` section, lines 347–578 and 583–807):

1. `open_picker_provider_sets_kind_and_mode`
2. `open_picker_keymap_resets_show_all`
3. `open_picker_noop_when_already_in_picker`
4. `picker_insert_char_updates_filter`
5. `picker_backspace_removes_from_filter`
6. `picker_confirm_provider_returns_provider_switch`
7. `picker_confirm_session_returns_session_load_command`
8. `picker_confirm_noop_with_no_active_picker`
9. `picker_confirm_strategy_updates_default`
10. `picker_move_up_decrements_selection`
11. `picker_move_down_increments_selection`
12. `picker_move_cursor_left_moves_cursor`
13. `picker_move_cursor_right_moves_cursor`
14. `toggle_keymap_scope_filter_toggles_flag`

**1 new test to add in the slice** (not moved, new):
- `picker_confirm_keymap_returns_intent_for_redispatch` — tests that `handle_picker_confirm` for keymap returns `(IntentResult::empty(), Some(Intent::Quit))` and sets mode to Normal

**1 test to keep in `nullslop-intent`** (tests the re-dispatch orchestration):
- `picker_confirm_keymap_sets_mode_and_signal` — this tests the full flow through `IntentHandler::handle()` → slice → re-dispatch, verifying `should_quit` is set

**2 tests that stay in `nullslop-intent`** (not picker intents):
- `session_new_creates_fresh_session`
- `session_new_noop_when_picker_active`

### What stays in `nullslop-intent`

- `SessionNew`, `RefreshModels`, `RescanPromptTemplates` match arms + handler functions
- The `picker_confirm_keymap` re-dispatch wrapper in the `PickerConfirm` match arm
- `handle_interrupt`, `handle_set_mode`, `handle_normal_escape` — cross-cutting handlers
- Import `nsslice_picker::validator` instead of `crate::validators::picker`
- Import `nsslice_picker::intent` for all 9 picker match arms

## Key Design Decision: `confirm_keymap` Return Type

The current `confirm_keymap` calls `IntentHandler::handle(&intent, state)` recursively. This creates a circular dependency if moved to the slice (`nullslop-intent` ↔ `nsslice-picker`).

**Solution**: Split into two parts:

1. **In `nsslice-picker/src/intent.rs`**: `handle_picker_confirm` returns `(IntentResult, Option<Intent>)`.
   - For Provider, ContextAssembly, Session: returns `(result, None)`
   - For Keymap: returns `(IntentResult::empty(), Some(selected_intent))` — does NOT re-dispatch

2. **In `nullslop-intent/src/handler.rs`**: The `PickerConfirm` match arm:
   ```rust
   Intent::PickerConfirm => {
       let (result, maybe_intent) = nsslice_picker::intent::handle_picker_confirm(state);
       if let Some(intent) = maybe_intent {
           // Re-dispatch the keymap intent (e.g., Quit).
           state.frontend.mode = Mode::Normal;
           let redispatch = IntentHandler::handle(&intent, state);
           IntentResult::with_commands([result.commands, redispatch.commands].concat())
       } else {
           result
       }
   }
   ```

Wait — actually, looking at the current code more carefully, `confirm_keymap` sets `state.frontend.mode = Mode::Normal` BEFORE calling `IntentHandler::handle()`. The re-dispatch then runs the full handler (which calls `state.frontend.tui_signals.clear()` again). So the mode is already set to Normal by the keymap confirm, then the re-dispatch happens.

For the slice, `handle_picker_confirm` for keymap should:
1. Set `state.frontend.mode = Mode::Normal` (this is part of the confirm logic)
2. Return `(IntentResult::empty(), Some(intent))`

And the caller in `nullslop-intent` will do the re-dispatch. The mode is already set, so the re-dispatch will just execute the intent (e.g., Quit sets `should_quit`).

Actually wait — `handle()` calls `state.frontend.tui_signals.clear()` at the top. If we set mode inside the slice and then re-dispatch from `nullslop-intent`, the re-dispatch will clear signals. That's fine — it's the same behavior as today (the recursive call also clears signals).

**Revised approach**: Keep it simpler. `handle_picker_confirm` for keymap:
- Sets `state.frontend.mode = Mode::Normal`
- Returns `(IntentResult::empty(), Some(selected_intent))`

The `nullslop-intent` caller then calls `IntentHandler::handle(&intent, state)` for the re-dispatch. This matches current behavior exactly.

## File Changes

### 1. NEW `crates/slices/nsslice-picker/Cargo.toml`

```toml
[package]
name = "nsslice-picker"
version = "0.1.0"
edition = "2024"

[dependencies]
nullslop-component = { workspace = true }
nullslop-protocol = { workspace = true }
wherror = { workspace = true }

[dev-dependencies]
rstest = { workspace = true }
jiff = { workspace = true }

[lints]
workspace = true
```

- `nullslop-component` — for `AppState`, picker state types, entry types
- `nullslop-protocol` — for `IntentResult`, `Command`, `Mode`, `PickerKind`, `Intent`
- `wherror` — for `#[derive(Error)]` on error enums
- `rstest` — test framework (dev-dep)
- `jiff` — needed by `SessionEntry` in tests (dev-dep)

### 2. NEW `crates/slices/nsslice-picker/src/lib.rs`

```rust
//! Picker slice — picker navigation, filtering, confirmation, and scope toggling.
//!
//! Handles all picker intents (open, insert char, backspace, confirm, move,
//! cursor movement, toggle scope filter) and their validators.
//! No element — rendering stays in `nullslop-tui`.

pub mod intent;
pub mod validator;
```

No `register()` — no UI element to register.

### 3. NEW `crates/slices/nsslice-picker/src/validator.rs`

Move entire contents of `nullslop-intent/src/validators/picker.rs`. Change imports from:
- `use crate::validators::picker::{OpenPickerError, PickerConfirmError};` → use `super::*` (same module now)

All 8 validator tests move with it. No other changes needed.

### 4. NEW `crates/slices/nsslice-picker/src/intent.rs`

Contains all handler functions and 14 handler tests. See the full file content in the implementation.

Key functions:
- `handle_open_picker(state: &mut AppState, kind: PickerKind) -> IntentResult`
- `handle_insert_char(state: &mut AppState, ch: char) -> IntentResult`
- `handle_backspace(state: &mut AppState) -> IntentResult`
- `handle_picker_confirm(state: &mut AppState) -> (IntentResult, Option<Intent>)`
- `handle_move_up(state: &mut AppState) -> IntentResult`
- `handle_move_down(state: &mut AppState) -> IntentResult`
- `handle_move_cursor_left(state: &mut AppState) -> IntentResult`
- `handle_move_cursor_right(state: &mut AppState) -> IntentResult`
- `handle_toggle_keymap_scope_filter(state: &mut AppState) -> IntentResult`
- `confirm_provider(state: &mut AppState) -> IntentResult` (private)
- `confirm_strategy(state: &mut AppState) -> IntentResult` (private)
- `confirm_keymap(state: &mut AppState) -> (IntentResult, Option<Intent>)` (private)
- `confirm_session(state: &mut AppState) -> IntentResult` (private)

Module-level constant: `const PICKER_MAX_VISIBLE: usize = 100;`

### 5. Root `Cargo.toml`

Add `nsslice-picker` to workspace members and workspace dependencies:

**members line** — add `"crates/slices/nsslice-picker"`:

```toml
members = ["crates/*", "crates/slices/nsslice-status-bar", ..., "crates/slices/nsslice-chat-input-box", "crates/slices/nsslice-picker", "actors/*", "tests/*"]
```

**[workspace.dependencies]** — add:
```toml
nsslice-picker = { workspace = true }
```

**[patch.crates-io]** or equivalent section — add:
```toml
nsslice-picker = { path = "crates/slices/nsslice-picker" }
```

### 6. `crates/nullslop-intent/Cargo.toml`

Add:
```toml
nsslice-picker = { workspace = true }
```

### 7. `crates/nullslop-intent/src/handler.rs`

**Remove import**: `use crate::validators::{app, chat_entry, picker};` → `use crate::validators::{app, chat_entry};`

**Add import**: No import needed — call via full path `nsslice_picker::intent::*` and `nsslice_picker::validator::*`.

Wait, looking at the current code, `picker::` is used both in match arms and in helper functions like `handle_open_picker`, `handle_toggle_keymap_scope_filter`. After extraction, these functions move to the slice. The handler only needs `nsslice_picker::intent::handle_*` calls in match arms.

**Replace 9 picker match arms**:

```rust
// --- Picker ---
Intent::OpenPicker { kind } => nsslice_picker::intent::handle_open_picker(state, *kind),
Intent::PickerInsertChar { ch } => nsslice_picker::intent::handle_insert_char(state, *ch),
Intent::PickerBackspace => nsslice_picker::intent::handle_backspace(state),
Intent::PickerConfirm => {
    let (result, maybe_intent) = nsslice_picker::intent::handle_picker_confirm(state);
    if let Some(intent) = maybe_intent {
        state.frontend.mode = Mode::Normal;
        let redispatch = IntentHandler::handle(&intent, state);
        IntentResult::with_commands([result.commands, redispatch.commands].concat())
    } else {
        result
    }
}
Intent::PickerMoveUp => nsslice_picker::intent::handle_move_up(state),
Intent::PickerMoveDown => nsslice_picker::intent::handle_move_down(state),
Intent::PickerMoveCursorLeft => nsslice_picker::intent::handle_move_cursor_left(state),
Intent::PickerMoveCursorRight => nsslice_picker::intent::handle_move_cursor_right(state),
Intent::ToggleKeymapScopeFilter => nsslice_picker::intent::handle_toggle_keymap_scope_filter(state),
```

Wait — but the keymap confirm already sets mode to Normal inside the slice. If we set it again in the handler, that's fine (idempotent). Actually, let me re-read the current `confirm_keymap`:

```rust
fn confirm_keymap(state: &mut AppState) -> IntentResult {
    let Some(entry) = state.frontend.keymap_picker.selected_item() else {
        return IntentResult::empty();
    };
    let intent = entry.command.clone();
    state.frontend.mode = Mode::Normal;
    IntentHandler::handle(&intent, state)
}
```

So `confirm_keymap` sets mode to Normal then re-dispatches. The re-dispatch calls `handle()` which clears tui_signals. The return value is the result of the re-dispatch.

In the new design:
- `nsslice_picker::intent::handle_picker_confirm` for keymap sets mode to Normal and returns `(IntentResult::empty(), Some(intent))`
- The caller in `nullslop-intent` does NOT need to set mode again. But it DOES need to call `IntentHandler::handle(&intent, state)`.

So the match arm should be:

```rust
Intent::PickerConfirm => {
    let (result, maybe_intent) = nsslice_picker::intent::handle_picker_confirm(state);
    if let Some(intent) = maybe_intent {
        let redispatch = IntentHandler::handle(&intent, state);
        IntentResult::with_commands([result.commands, redispatch.commands].concat())
    } else {
        result
    }
}
```

Note: no mode setting here — that's already done inside the slice for keymap. For other picker kinds, mode is set inside `confirm_provider`, `confirm_strategy`, `confirm_session`.

**Remove from handler.rs**:
- The `// --- Picker handlers ---` section and all 7 functions:
  - `handle_open_picker`
  - `handle_picker_confirm`
  - `confirm_provider`
  - `confirm_strategy`
  - `confirm_keymap`
  - `confirm_session`
  - `handle_toggle_keymap_scope_filter`
- `PICKER_MAX_VISIBLE` constant from `IntentHandler`

### 8. `crates/nullslop-intent/src/validators/picker.rs` — DELETE

### 9. `crates/nullslop-intent/src/validators/mod.rs` — Remove `pub mod picker;`

```rust
pub mod app;
pub mod chat_entry;
```

### 10. `crates/nullslop-intent/src/handler_tests.rs`

**Remove 14 picker tests** (lines 347–578 and 583–807), splitting around `picker_confirm_keymap_sets_mode_and_signal` which stays:

Tests to remove (move to slice):
1. `open_picker_provider_sets_kind_and_mode`
2. `open_picker_keymap_resets_show_all`
3. `open_picker_noop_when_already_in_picker`
4. `picker_insert_char_updates_filter`
5. `picker_backspace_removes_from_filter`
6. `picker_confirm_provider_returns_provider_switch`
7. `picker_confirm_session_returns_session_load_command`
8. `picker_confirm_noop_with_no_active_picker`
9. `picker_confirm_strategy_updates_default`
10. `picker_move_up_decrements_selection`
11. `picker_move_down_increments_selection`
12. `picker_move_cursor_left_moves_cursor`
13. `picker_move_cursor_right_moves_cursor`
14. `toggle_keymap_scope_filter_toggles_flag`

Tests to keep in `nullslop-intent/src/handler_tests.rs`:
1. `picker_confirm_keymap_sets_mode_and_signal` — tests full re-dispatch flow through IntentHandler
2. `session_new_creates_fresh_session` — tests SessionNew intent (stays in nullslop-intent)
3. `session_new_noop_when_picker_active` — tests SessionNew intent (stays in nullslop-intent)

The 3 kept tests stay in the "Picker Intents" section comment area, or can be re-grouped into a "SessionNew" subsection. Simpler to just leave them in place.

## Acceptance Criteria

1. `crates/slices/nsslice-picker/` exists with `Cargo.toml`, `src/lib.rs`, `src/validator.rs`, `src/intent.rs`
2. `nsslice-picker` is a workspace member in root `Cargo.toml`
3. `nullslop-intent/Cargo.toml` has `nsslice-picker` dependency
4. `nullslop-intent/src/validators/picker.rs` is deleted
5. `nullslop-intent/src/validators/mod.rs` has no `pub mod picker;`
6. `nullslop-intent/src/handler.rs` has 9 picker match arms delegating to `nsslice_picker::intent::*`
7. `nullslop-intent/src/handler.rs` has `PickerConfirm` handling the `Option<Intent>` return for keymap re-dispatch
8. `nullslop-intent/src/handler.rs` no longer has picker helper functions or `PICKER_MAX_VISIBLE`
9. `nsslice-picker` has 14 handler tests + 8 validator tests, all passing independently (`cargo test -p nsslice-picker`)
10. `nullslop-intent` has the keymap confirm re-dispatch test still passing
11. `cargo test --workspace` passes — no regressions

## Risks

- **`confirm_keymap` return type change** — the `(IntentResult, Option<Intent>)` pattern is new. The keymap confirm test in `nullslop-intent` will verify the re-dispatch still works.
- **`jiff` dev-dependency** — `nsslice-picker` tests create `SessionEntry` which uses `jiff::Timestamp`. Need `jiff` as dev-dep.
- **Import changes** — validator tests in the old file used `crate::validators::picker::{...}`. In the slice, they'll use `super::*`. Need to verify all test imports.
- **`Intent` type visibility** — `confirm_keymap` returns `Option<Intent>`. `Intent` must be importable from `nullslop-protocol`. It already is (it's in the protocol crate).
