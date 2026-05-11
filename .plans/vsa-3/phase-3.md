# Phase 3: Extract Global → `nsslice-global`

## Problem

Three global intents (`Quit`, `ToggleWhichkey`, `Interrupt`) are handled inline or via a helper function in `nullslop-intent/src/handler.rs`, with validators split between `nullslop-intent/src/validators/app.rs` and `nsslice-chat-input-box/src/validator.rs`. This phase creates a dedicated `nsslice-global` slice crate, consolidates all three validators into it, and moves the `cancel_stream_and_drain` helper so it can be shared with `SetMode` (which stays in `nullslop-intent` until VSA-4).

This is the most complex phase of VSA-3 because it:
1. **Moves validators from two different crates** into one new crate
2. **Moves a shared helper** (`cancel_stream_and_drain`) that `nullslop-intent` still needs
3. **Deletes the entire `validators/` directory** from `nullslop-intent`
4. **Leaves `handle_normal_escape` and `handle_set_mode` in `nullslop-intent`** but wires them to call `nsslice_global`

## What Moves

### Validators → `nsslice-global/src/validator.rs`

**From `nullslop-intent/src/validators/app.rs` (entire file → deleted):**
1. `validate_quit(_state) -> ()` — infallible
2. `validate_toggle_whichkey(_state) -> ()` — infallible
3. `validate_normal_escape(_state) -> ()` — infallible

**From `nsslice-chat-input-box/src/validator.rs` (partial extract):**
4. `InterruptError` enum (1 variant: `NothingToInterrupt`)
5. `validate_interrupt(state) -> Result<(), InterruptError>` — checks empty buffer + idle session
6. 3 interrupt tests: `interrupt_succeeds_with_non_empty_buffer`, `interrupt_succeeds_with_active_stream`, `interrupt_fails_with_empty_buffer_and_idle_session`

### Handlers → `nsslice-global/src/intent.rs`

**From `nullslop-intent/src/handler.rs` (3 match arms + 1 helper function):**
1. `handle_quit(state) -> IntentResult` — validate + set `should_quit`
2. `handle_toggle_whichkey(state) -> IntentResult` — validate + set signal
3. `handle_interrupt(state) -> IntentResult` — validate + deactivate autocomplete + cancel stream or reset input
4. `cancel_stream_and_drain(state)` — public helper, shared with `SetMode` (stays in `nullslop-intent`)

**From `nullslop-intent/src/handler_tests.rs` (6 tests → removed):**
1. `quit_sets_should_quit`
2. `toggle_whichkey_sets_tui_signal`
3. `interrupt_resets_non_empty_buffer`
4. `interrupt_cancels_stream_when_buffer_empty`
5. `interrupt_noop_when_idle_and_empty`
6. `interrupt_drains_queued_messages_to_input_buffer`

### What stays in `nullslop-intent`

- `handle_set_mode` — updated to call `nsslice_global::intent::cancel_stream_and_drain(state)` instead of the local function
- `handle_normal_escape` — updated to call `nsslice_global::validator::validate_normal_escape(state)` instead of `app::validate_normal_escape(state)`
- Picker re-dispatch logic for keymap confirm
- `tui_signals.clear()` preamble
- 8 remaining tests in `handler_tests.rs` (set_mode × 4, normal_escape × 2, picker_confirm × 1, tui_signals × 1)

### Cleanups after the move

- Delete `nullslop-intent/src/validators/app.rs`
- Delete `nullslop-intent/src/validators/mod.rs`
- Remove `pub mod validators;` from `nullslop-intent/src/lib.rs`
- Remove `use crate::validators::app;` from `handler.rs`
- Remove `wherror` from `nullslop-intent/Cargo.toml` (no longer used)
- Update `nsslice-chat-input-box/src/validator.rs` — remove `InterruptError`, `validate_interrupt`, 3 interrupt tests, update module doc comment

## File Changes

### 1. NEW `crates/slices/nsslice-global/Cargo.toml`

```toml
[package]
name = "nsslice-global"
version = "0.1.0"
edition = "2024"

[dependencies]
nullslop-component = { workspace = true }
nullslop-protocol = { workspace = true }
wherror = { workspace = true }

[dev-dependencies]
rstest = { workspace = true }

[lints]
workspace = true
```

- `nullslop-component` — for `AppState`, `ChatSessionState` methods
- `nullslop-protocol` — for `IntentResult`, `Command`, `CancelStream`
- `wherror` — for `#[derive(Error)]` on `InterruptError`
- No dependency on `nsslice-chat-input-box` — all access goes through `AppState`

### 2. NEW `crates/slices/nsslice-global/src/lib.rs`

```rust
//! Global slice — quit, toggle which-key, and interrupt.
//!
//! Handles cross-cutting application actions: quitting, toggling the
//! which-key popup, and interrupting the active stream or clearing input.
//! No element — rendering stays in `nullslop-tui`.

pub mod intent;
pub mod validator;
```

No element — no `register()`.

### 3. NEW `crates/slices/nsslice-global/src/validator.rs`

Consolidates validators from two sources:

**Imports:**
```rust
use nullslop_component::AppState;
use wherror::Error;
```

**Contents (in order):**
1. Module doc comment: "Global intent validators — quit, toggle which-key, interrupt, and normal escape."
2. `validate_quit` — infallible, from `app.rs`
3. `validate_toggle_whichkey` — infallible, from `app.rs`
4. `validate_normal_escape` — infallible, from `app.rs` (handler stays in `nullslop-intent` until VSA-4)
5. `InterruptError` enum — from `nsslice-chat-input-box/validator.rs`
6. `validate_interrupt` — from `nsslice-chat-input-box/validator.rs`
7. 3 interrupt tests (moved from `nsslice-chat-input-box/validator.rs`)

No tests for the 3 infallible validators (they just return `()`) — same as before.

### 4. NEW `crates/slices/nsslice-global/src/intent.rs`

**Imports:**
```rust
use nullslop_component::AppState;
use nullslop_protocol::provider::CancelStream;
use nullslop_protocol::{Command, IntentResult};

use crate::validator;
```

**Contents:**
1. `handle_quit(state)` — calls `validator::validate_quit(state)`, sets `should_quit`
2. `handle_toggle_whichkey(state)` — calls `validator::validate_toggle_whichkey(state)`, sets signal
3. `handle_interrupt(state)` — calls `validator::validate_interrupt(state)`, deactivates autocomplete, cancels stream or resets input
4. `pub fn cancel_stream_and_drain(state)` — moved from `nullslop-intent`, shared with `SetMode`
5. 6 handler tests (moved from `handler_tests.rs`)

### 5. Root `Cargo.toml`

**members line** — add `"crates/slices/nsslice-global"`:

```
members = [..., "crates/slices/nsslice-session-management", "crates/slices/nsslice-global", "actors/*", "tests/*"]
```

**[workspace.dependencies]** — add:
```toml
nsslice-global = { path = "crates/slices/nsslice-global" }
```

**[dependencies]** — add:
```toml
nsslice-global = { workspace = true }
```

### 6. `crates/nullslop-intent/Cargo.toml`

Add `nsslice-global` dependency:
```toml
nsslice-global = { workspace = true }
```

Remove `wherror` dependency (no longer used by any source in the crate):
```toml
# Remove this line:
wherror = { workspace = true }
```

### 7. `crates/nullslop-intent/src/handler.rs`

**Replace 3 match arms** with delegations:

```rust
// Old:
Intent::Quit => {
    app::validate_quit(state);
    state.frontend.should_quit = true;
    IntentResult::empty()
}
Intent::Interrupt => handle_interrupt(state),
// ...
Intent::ToggleWhichkey => {
    app::validate_toggle_whichkey(state);
    state.frontend.tui_signals.toggle_whichkey = true;
    IntentResult::empty()
}

// New:
Intent::Quit => nsslice_global::intent::handle_quit(state),
Intent::Interrupt => nsslice_global::intent::handle_interrupt(state),
// ...
Intent::ToggleWhichkey => nsslice_global::intent::handle_toggle_whichkey(state),
```

**Remove 2 free functions:** `handle_interrupt`, `cancel_stream_and_drain`.

**Update `handle_set_mode`** to call `nsslice_global::intent::cancel_stream_and_drain(state)`:
```rust
// Old:
cancel_stream_and_drain(state);

// New:
nsslice_global::intent::cancel_stream_and_drain(state);
```

**Update `handle_normal_escape`** to call `nsslice_global::validator::validate_normal_escape(state)`:
```rust
// Old:
app::validate_normal_escape(state);

// New:
nsslice_global::validator::validate_normal_escape(state);
```

**Clean up imports:**

Remove `app` import (no longer used — `handle_normal_escape` now calls `nsslice_global`):
```rust
// Old:
use crate::validators::app;

// New:
// (remove entirely)
```

All other imports remain used:
- `AppState` — used by `handle_set_mode`, `handle_normal_escape`
- `CancelStream` — used by `handle_set_mode`
- `Command` — used by `handle_set_mode`
- `Mode` — used by `handle_set_mode`
- `PinPosition` — used by PinnedPanel match arms
- `IntentResult` — used by match arms and `handle_set_mode`/`handle_normal_escape`

### 8. `crates/nullslop-intent/src/handler_tests.rs`

**Remove 7 tests** (quit, toggle_whichkey, and 4 interrupt + the tui_signals test that triggers via Quit... wait, actually `tui_signals_are_cleared_at_start_of_handle` tests the IntentHandler preamble, not any specific intent. It uses `Quit` as the trigger but the test is about the dispatch hub. This test should STAY.)

Remove 6 tests:
1. `quit_sets_should_quit`
2. `toggle_whichkey_sets_tui_signal`
3. `interrupt_resets_non_empty_buffer`
4. `interrupt_cancels_stream_when_buffer_empty`
5. `interrupt_noop_when_idle_and_empty`
6. `interrupt_drains_queued_messages_to_input_buffer`

**Keep 8 tests:**
1. `set_mode_changes_mode`
2. `set_mode_clears_picker_kind_when_leaving_picker`
3. `normal_escape_clears_selection`
4. `normal_escape_sets_close_signal_even_without_selection`
5. `picker_confirm_keymap_sets_mode_and_signal`
6. `tui_signals_are_cleared_at_start_of_handle` — tests the `IntentHandler::handle` preamble, uses `Quit` as trigger but works through delegation
7. `set_mode_input_to_normal_during_streaming_cancels_stream`
8. `set_mode_input_to_normal_during_streaming_drains_queue`

**No import cleanup needed** — all imports (`KeymapEntry`, `AppState`, `FrontendState`, `ChatEntry`, `Command`, `Mode`, `PickerKind`) remain used by the 8 remaining tests.

### 9. Delete `nullslop-intent/src/validators/app.rs`

### 10. Delete `nullslop-intent/src/validators/mod.rs`

### 11. Update `crates/nullslop-intent/src/lib.rs`

Remove `pub mod validators;` — the directory is gone.

### 12. Update `nsslice-chat-input-box/src/validator.rs`

Remove `InterruptError`, `validate_interrupt`, and the 3 interrupt tests. Update the module doc comment.

```rust
// Old doc:
//! Chat input intent validators.
//!
//! Validators for message submission, autocomplete confirmation, and interrupt intents.

// New doc:
//! Chat input intent validators.
//!
//! Validators for message submission and autocomplete confirmation.
```

Remove these items:
- `InterruptError` enum
- `validate_interrupt` function
- 3 interrupt tests in `mod tests`: `interrupt_succeeds_with_non_empty_buffer`, `interrupt_succeeds_with_active_stream`, `interrupt_fails_with_empty_buffer_and_idle_session`

After removal, the `wherror` import is still needed by `SubmitMessageError` and `AutocompleteConfirmError`.

## Implementation Order

1. Create `crates/slices/nsslice-global/Cargo.toml`
2. Create `crates/slices/nsslice-global/src/lib.rs`
3. Create `crates/slices/nsslice-global/src/validator.rs` — 3 infallible validators + `InterruptError` + `validate_interrupt` + 3 tests
4. Create `crates/slices/nsslice-global/src/intent.rs` — 3 handler functions + `cancel_stream_and_drain` + 6 tests
5. Add `nsslice-global` to root `Cargo.toml` (members, workspace.dependencies, dependencies)
6. Add `nsslice-global` dep to `nullslop-intent/Cargo.toml`; remove `wherror` dep
7. Update `nullslop-intent/src/handler.rs` — replace 3 match arms, remove 2 free functions, update `handle_set_mode` and `handle_normal_escape`, clean imports
8. Update `nullslop-intent/src/handler_tests.rs` — remove 6 tests
9. Delete `nullslop-intent/src/validators/app.rs`
10. Delete `nullslop-intent/src/validators/mod.rs`
11. Update `nullslop-intent/src/lib.rs` — remove `pub mod validators;`
12. Update `nsslice-chat-input-box/src/validator.rs` — remove interrupt items, update doc comment
13. Run `cargo test --workspace`

## Acceptance Criteria

1. `crates/slices/nsslice-global/` exists with `Cargo.toml`, `src/lib.rs`, `src/validator.rs`, `src/intent.rs`
2. `nsslice-global` is a workspace member in root `Cargo.toml`
3. `nullslop-intent/Cargo.toml` has `nsslice-global` dependency and no longer has `wherror`
4. `nsslice-global` has 3 validator tests + 6 handler tests (9 total), all passing independently
5. `nsslice-chat-input-box` no longer has `InterruptError`, `validate_interrupt`, or interrupt tests; module doc updated
6. `nullslop-intent/src/handler.rs` delegates Quit/ToggleWhichkey/Interrupt to `nsslice_global::intent::*`
7. `handle_set_mode` calls `nsslice_global::intent::cancel_stream_and_drain(state)`
8. `handle_normal_escape` calls `nsslice_global::validator::validate_normal_escape(state)`
9. `nullslop-intent/src/validators/` directory no longer exists
10. `nullslop-intent/src/lib.rs` no longer has `pub mod validators;`
11. `nullslop-intent/src/handler_tests.rs` has 8 remaining tests (no import cleanup needed)
12. `cargo test --workspace` passes — no regressions
