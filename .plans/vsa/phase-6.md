# Phase 6: Extract `nsslice-chat-input-box`

This phase extracts the chat input box feature — element, validators, and intent
handlers — into a new slice crate at `crates/slices/nsslice-chat-input-box/`. This is
the second intent-bearing slice, following the pattern established by `nsslice-pinned-panel`
in Phase 5.

## Context

The chat input box feature currently spans 3 locations:
- **Element**: `nullslop-component/src/chat_input_box/element.rs` — rendering (11 tests)
- **State**: `nullslop-component/src/chat_input_box/state.rs` — stays in place
- **Validators**: `nullslop-intent/src/validators/chat_input.rs` — 3 validators + 3 error types (8 tests)
- **Handlers**: `nullslop-intent/src/handler.rs` — 5 handler functions + 3 helpers + 13 inline match arms

### Key decision: `handle_interrupt` stays in `nullslop-intent`

The high-level plan proposed splitting `handle_interrupt` into a slice part
(`handle_interrupt_input`) and an orchestration part (CancelStream). After analysis,
this creates unnecessary complexity:

1. `handle_interrupt` is cross-cutting: it validates, deactivates autocomplete, checks
   buffer state, calls `cancel_stream_and_drain`, and emits `CancelStream`.
2. `cancel_stream_and_drain` is shared between `handle_interrupt` and `handle_set_mode`
   — both stay in `nullslop-intent`.
3. Splitting would require a non-standard `(IntentResult, bool)` return type.

Instead, `handle_interrupt` stays in `nullslop-intent` and calls the slice's validator:
`nsslice_chat_input_box::validator::validate_interrupt(state)`. The 4 interrupt handler
tests stay in `nullslop-intent`'s `handler_tests.rs` since they test cross-cutting behavior.

This matches the "Cross-cutting handlers stay in `nullslop-intent`" convention from the
high-level plan.

### Dependency chain

The slice crate depends on:
- `nullslop-component` — for `AppState`, `AppUiRegistry`, `chat_input_box::AutocompleteMatch`,
  `prompt_template::PromptTemplateStore`, `FrontendState`
- `nullslop-component-ui` — for `UiElement` trait
- `nullslop-protocol` — for `IntentResult`, `Command`, `Mode`,
  `chat_input::EnqueueUserMessage`
- `ratatui` — for element rendering
- `wherror` — for validator error types
- `unicode-segmentation` — for grapheme iteration in `handle_insert_char`

`nullslop-intent` gains a dependency on `nsslice-chat-input-box` so:
- Match arms can call `nsslice_chat_input_box::intent::*()` for 13 intents
- `handle_interrupt` can call `nsslice_chat_input_box::validator::validate_interrupt()`

## Detailed Steps

### 1. Create `crates/slices/nsslice-chat-input-box/Cargo.toml`

```toml
[package]
name = "nsslice-chat-input-box"
version = "0.1.0"
edition = "2024"

[dependencies]
nullslop-component-ui = { workspace = true }
nullslop-component = { workspace = true }
nullslop-protocol = { workspace = true }
ratatui = { workspace = true }
wherror = { workspace = true }
unicode-segmentation = { workspace = true }

[dev-dependencies]
rstest = { workspace = true }

[lints]
workspace = true
```

### 2. Create `crates/slices/nsslice-chat-input-box/src/lib.rs`

```rust
//! Chat input box slice — message composition UI, validation, and intent handling.
//!
//! Co-locates everything about the chat input box:
//!
//! - **Element** — renders the input prompt with cursor positioning and mode-aware styling.
//! - **Validator** — validates message submission, autocomplete confirmation, and interrupt.
//! - **Intent** — handles 13 chat-input intents (character insertion, deletion, submission,
//!   autocomplete, cursor movement).
//!
//! State (`ChatInputBoxState`) stays in `nullslop-component` to avoid circular dependencies.
//!
//! **Note**: `handle_interrupt` and `handle_set_mode` stay in `nullslop-intent` because
//! they're cross-cutting (cancel streams, transition modes). They call into this slice's
//! validators but orchestrate domain logic themselves.

pub mod element;
pub mod intent;
pub mod validator;

pub use element::ChatInputBoxElement;

use nullslop_component::AppUiRegistry;

/// Register chat input box UI element.
pub fn register(registry: &mut AppUiRegistry) {
    registry.register(Box::new(ChatInputBoxElement));
}
```

### 3. Create `crates/slices/nsslice-chat-input-box/src/element.rs`

Copy from `nullslop-component/src/chat_input_box/element.rs` with import changes:

```rust
// Before:
use crate::AppState;

// After:
use nullslop_component::AppState;
```

In tests:
```rust
// Before:
use crate::FrontendState;

// After:
use nullslop_component::FrontendState;
```

All other imports (`nullslop_component_ui::UiElement`, `nullslop_protocol::Mode`,
`ratatui::*`) remain unchanged — they were already absolute.

Move all 11 element tests:
1. `name_returns_chat_input_box`
2. `render_draws_input_buffer`
3. `render_input_mode_yellow_prompt`
4. `render_input_mode_yellow_border`
5. `render_input_mode_cursor_at_end_of_text`
6. `render_cursor_at_mid_buffer`
7. `render_cursor_at_home`
8. `multiline_first_line_has_prefix`
9. `multiline_second_line_has_indent`
10. `render_multiline_cursor_on_second_line`
11. `render_multiline_cursor_between_newlines`

### 4. Create `crates/slices/nsslice-chat-input-box/src/validator.rs`

Copy from `nullslop-intent/src/validators/chat_input.rs`. Import changes:

```rust
// Before:
use nullslop_component::AppState;

// After: (no change — already uses nullslop_component::AppState)
```

Move all 8 validator tests (3 for SubmitMessage, 2 for AutocompleteConfirm, 3 for Interrupt).

### 5. Create `crates/slices/nsslice-chat-input-box/src/intent.rs`

This file contains the handler functions and their tests. Move from `handler.rs`:

**Handler functions moved (5 existing):**
- `handle_insert_char(ch: char, state: &mut AppState) -> IntentResult`
- `handle_delete_grapheme(state: &mut AppState) -> IntentResult`
- `handle_delete_grapheme_forward(state: &mut AppState) -> IntentResult`
- `handle_submit_message(state: &mut AppState) -> IntentResult`
- `handle_autocomplete_confirm(state: &mut AppState) -> IntentResult`

**New thin wrappers for the 8 inline cursor movement match arms:**
- `handle_move_cursor_left(state: &mut AppState) -> IntentResult`
- `handle_move_cursor_right(state: &mut AppState) -> IntentResult`
- `handle_move_cursor_to_start(state: &mut AppState) -> IntentResult`
- `handle_move_cursor_to_end(state: &mut AppState) -> IntentResult`
- `handle_move_cursor_word_left(state: &mut AppState) -> IntentResult`
- `handle_move_cursor_word_right(state: &mut AppState) -> IntentResult`
- `handle_move_cursor_up(state: &mut AppState) -> IntentResult`
- `handle_move_cursor_down(state: &mut AppState) -> IntentResult`

**Helper functions moved (3):**
- `is_valid_trigger_position(input: &ChatInputBoxState) -> bool`
- `should_deactivate_on_cursor_move(state: &AppState) -> bool`
- `compute_matches(store: &PromptTemplateStore, filter: &str) -> Vec<AutocompleteMatch>`

**Imports:**
```rust
use nullslop_component::AppState;
use nullslop_component::chat_input_box::AutocompleteMatch;
use nullslop_component::prompt_template::PromptTemplateStore;
use nullslop_protocol::{Command, IntentResult};
use nullslop_protocol::chat_input::EnqueueUserMessage;
use unicode_segmentation::UnicodeSegmentation as _;

use crate::validator;
```

Note: `is_valid_trigger_position` currently takes `&nullslop_component::chat_input_box::ChatInputBoxState`
as its parameter type. After moving, the import becomes `use nullslop_component::chat_input_box::ChatInputBoxState;`
or the function can keep the fully-qualified path.

**Tests:** Move 14 handler tests from `nullslop-intent/src/handler_tests.rs`
(Chat Input Intents section, lines 34-237):

1. `insert_char_appends_to_buffer`
2. `delete_grapheme_removes_last_char`
3. `delete_grapheme_forward_removes_next_char`
4. `submit_message_returns_enqueue_command`
5. `submit_message_noop_with_empty_buffer`
6. `autocomplete_confirm_falls_back_to_switch_tab`
7. `move_cursor_left_moves_cursor`
8. `move_cursor_right_moves_cursor`
9. `move_cursor_to_start_moves_cursor`
10. `move_cursor_to_end_moves_cursor`
11. `move_cursor_word_left_moves_cursor`
12. `move_cursor_word_right_moves_cursor`
13. `move_cursor_up_delegates_to_state`
14. `move_cursor_down_delegates_to_state`

**Important**: Tests call slice functions directly (e.g., `super::handle_insert_char('x', &mut state)`)
instead of going through `IntentHandler::handle()`, following the Phase 5 convention.

The `handle()` helper function from handler_tests.rs is NOT moved — tests call handler
functions directly.

**Note**: The `autocomplete_confirm_falls_back_to_switch_tab` test asserts that
`state.frontend.active_tab` changes. This works because the slice has access to
`AppState` which includes `frontend.active_tab`. No import of `Intent` is needed
since we're calling the function directly.

### 6. Delete `nullslop-component/src/chat_input_box/element.rs`

After copying to the slice, delete the element file.

### 7. Update `nullslop-component/src/chat_input_box/mod.rs`

Remove `pub mod element;` and `pub use element::ChatInputBoxElement;`. Keep only:
```rust
//! Chat input box — where the user composes and sends messages.
//!
//! This component manages the text input experience end to end: handling keystrokes,
//! displaying the in-progress message, tracking the input buffer, and switching
//! between browsing and typing modes.
//!
//! The rendering element and intent handling are in the `nsslice-chat-input-box` slice crate.
//! Only state types remain here.

pub mod state;

pub use state::{AutocompleteMatch, AutocompleteState, ChatInputBoxState};
```

### 8. Remove `ChatInputBoxElement` from `register_tui_elements()`

In `nullslop-component/src/lib.rs`, remove the line:
```rust
registry.register(Box::new(chat_input_box::ChatInputBoxElement));
```

### 9. Update `nullslop-intent/src/handler.rs`

**Remove imports (now unused after handler code moves):**
- `use nullslop_component::chat_input_box::AutocompleteMatch;`
- `use nullslop_component::prompt_template::PromptTemplateStore;`
- `use unicode_segmentation::UnicodeSegmentation as _;`
- `use crate::validators::chat_input;` (replace with slice calls where needed)

**Remove handler functions + helpers (8 items):**
- `handle_insert_char`
- `handle_delete_grapheme`
- `handle_delete_grapheme_forward`
- `handle_submit_message`
- `handle_autocomplete_confirm`
- `is_valid_trigger_position`
- `should_deactivate_on_cursor_move`
- `compute_matches`

**Update match arms** to call slice functions:

```rust
// --- Chat Input ---
Intent::InsertChar { ch } => {
    nsslice_chat_input_box::intent::handle_insert_char(*ch, state)
}
Intent::DeleteGrapheme => {
    nsslice_chat_input_box::intent::handle_delete_grapheme(state)
}
Intent::DeleteGraphemeForward => {
    nsslice_chat_input_box::intent::handle_delete_grapheme_forward(state)
}
Intent::SubmitMessage => {
    nsslice_chat_input_box::intent::handle_submit_message(state)
}
Intent::AutocompleteConfirm => {
    nsslice_chat_input_box::intent::handle_autocomplete_confirm(state)
}
Intent::MoveCursorLeft => {
    nsslice_chat_input_box::intent::handle_move_cursor_left(state)
}
Intent::MoveCursorRight => {
    nsslice_chat_input_box::intent::handle_move_cursor_right(state)
}
Intent::MoveCursorToStart => {
    nsslice_chat_input_box::intent::handle_move_cursor_to_start(state)
}
Intent::MoveCursorToEnd => {
    nsslice_chat_input_box::intent::handle_move_cursor_to_end(state)
}
Intent::MoveCursorWordLeft => {
    nsslice_chat_input_box::intent::handle_move_cursor_word_left(state)
}
Intent::MoveCursorWordRight => {
    nsslice_chat_input_box::intent::handle_move_cursor_word_right(state)
}
Intent::MoveCursorUp => {
    nsslice_chat_input_box::intent::handle_move_cursor_up(state)
}
Intent::MoveCursorDown => {
    nsslice_chat_input_box::intent::handle_move_cursor_down(state)
}
```

**Update `handle_interrupt`** — change validator call:

```rust
// Before:
if chat_input::validate_interrupt(state).is_err() {

// After:
if nsslice_chat_input_box::validator::validate_interrupt(state).is_err() {
```

`handle_interrupt`, `handle_set_mode`, and `cancel_stream_and_drain` stay in
`nullslop-intent` — they're cross-cutting.

### 10. Delete `nullslop-intent/src/validators/chat_input.rs`

### 11. Update `nullslop-intent/src/validators/mod.rs`

Remove `pub mod chat_input;`

### 12. Move chat-input handler tests from `handler_tests.rs`

Remove the 14 test functions from the `// Chat Input Intents` section in
`nullslop-intent/src/handler_tests.rs` (lines 34-237). These tests move to
`nsslice-chat-input-box/src/intent.rs`.

**Do NOT remove** the 4 interrupt tests from the `// Mode & App Intents` section:
- `interrupt_resets_non_empty_buffer`
- `interrupt_cancels_stream_when_buffer_empty`
- `interrupt_noop_when_idle_and_empty`
- `interrupt_drains_queued_messages_to_input_buffer`

These stay in `nullslop-intent` because they test `handle_interrupt`, which is
cross-cutting and stays in `nullslop-intent`.

### 13. Add workspace entries in root `Cargo.toml`

**`[workspace] members`** — add `"crates/slices/nsslice-chat-input-box"` to the list.

**`[workspace.dependencies]`** — add:
```toml
nsslice-chat-input-box = { path = "crates/slices/nsslice-chat-input-box" }
```

**`[dependencies]`** (root package) — add:
```toml
nsslice-chat-input-box = { workspace = true }
```

### 14. Wire registration in `src/app.rs`

Add `nsslice_chat_input_box::register(&mut ui_registry)` at both registration sites,
after the existing slice registrations:

**TUI path** (~line 145):
```rust
nsslice_pinned_panel::register(&mut ui_registry);
nsslice_chat_input_box::register(&mut ui_registry);  // ADD
```

**Headless path** (~line 516):
```rust
nsslice_pinned_panel::register(&mut registry);
nsslice_chat_input_box::register(&mut registry);  // ADD
```

### 15. Add `nsslice-chat-input-box` dependency to `crates/nullslop-tui/Cargo.toml`

Add `nsslice-chat-input-box = { workspace = true }` to dependencies.

### 16. Add registration in `crates/nullslop-tui/src/app.rs`

Add `nsslice_chat_input_box::register(&mut ui_registry)` at all 3 registration sites,
after the existing `nsslice_pinned_panel::register()` calls.

### 17. Add `nsslice-chat-input-box` dependency to `crates/nullslop-intent/Cargo.toml`

Add `nsslice-chat-input-box = { workspace = true }` to dependencies.

### 18. Verify

```bash
cargo check --workspace
cargo test --workspace
cargo test -p nsslice-chat-input-box
cargo test -p nullslop-intent
```

## Acceptance Criteria

1. **No duplicated code**: `nullslop-component/src/chat_input_box/element.rs` and
   `nullslop-intent/src/validators/chat_input.rs` are deleted. Handler functions and
   helpers removed from `nullslop-intent/src/handler.rs`.
2. **No orphaned registrations**: `register_tui_elements()` in `nullslop-component/src/lib.rs`
   does not register `ChatInputBoxElement`.
3. **Slice registered everywhere**: `nsslice_chat_input_box::register()` called at all 5 sites
   (2 in `src/app.rs`, 3 in `crates/nullslop-tui/src/app.rs`).
4. **Intent dispatch wired**: `nullslop-intent/src/handler.rs` calls into
   `nsslice_chat_input_box::intent::*()` for all 13 chat-input intents.
5. **Cross-cutting handlers intact**: `handle_interrupt` and `handle_set_mode` remain in
   `nullslop-intent/src/handler.rs`. `handle_interrupt` calls
   `nsslice_chat_input_box::validator::validate_interrupt()`.
6. **All tests moved**: 14 handler tests + 8 validator tests + 11 element tests
   run in `nsslice-chat-input-box`. None of those 33 remain in `nullslop-intent`.
   The 4 interrupt handler tests stay in `nullslop-intent` (they test cross-cutting behavior).
7. **State stays**: `ChatInputBoxState`, `AutocompleteMatch`, `AutocompleteState` remain in
   `nullslop-component/src/chat_input_box/state.rs` and are re-exported from the module.
8. **No circular dependencies**: `nsslice-chat-input-box` depends on `nullslop-component`,
   not vice versa. `nullslop-intent` depends on `nsslice-chat-input-box`, not vice versa.
9. **Full workspace tests pass**: `cargo test --workspace` passes with no failures.
10. **Independently testable**: `cargo test -p nsslice-chat-input-box` passes.

## Files Changed

### Created
- `crates/slices/nsslice-chat-input-box/Cargo.toml`
- `crates/slices/nsslice-chat-input-box/src/lib.rs`
- `crates/slices/nsslice-chat-input-box/src/element.rs`
- `crates/slices/nsslice-chat-input-box/src/validator.rs`
- `crates/slices/nsslice-chat-input-box/src/intent.rs`

### Deleted
- `crates/nullslop-component/src/chat_input_box/element.rs`
- `crates/nullslop-intent/src/validators/chat_input.rs`

### Modified
- `Cargo.toml` (root) — workspace members + dependencies
- `crates/nullslop-component/src/chat_input_box/mod.rs` — remove element submodule + re-export
- `crates/nullslop-component/src/lib.rs` — remove `ChatInputBoxElement` registration
- `crates/nullslop-intent/src/handler.rs` — remove handlers, delegate to slice, update interrupt validator call
- `crates/nullslop-intent/src/handler_tests.rs` — remove 14 chat-input tests
- `crates/nullslop-intent/src/validators/mod.rs` — remove `chat_input` module
- `crates/nullslop-intent/Cargo.toml` — add `nsslice-chat-input-box` dependency
- `crates/nullslop-tui/Cargo.toml` — add `nsslice-chat-input-box` dependency
- `crates/nullslop-tui/src/app.rs` — add registration at 3 sites
- `src/app.rs` — add registration at 2 sites
