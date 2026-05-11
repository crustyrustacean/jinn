# Phase 3: Extract Chat Entry Selection → `nsslice-chat-entry-selection`

## Problem

Three chat entry selection intents (`ChatEntrySelectNext`, `ChatEntrySelectPrev`, `ChatEntryPinSelected`) plus their validators and handler helper function still live in `nullslop-intent`. This phase extracts them into a dedicated `nsslice-chat-entry-selection` slice crate. Like `nsslice-picker`, this slice has no element (no rendering) — it's purely intent handling and validation.

This is the simplest extraction in VSA-2: 3 intents, 3 validators (1 fallible), 4 handler tests, 3 validator tests. No circular dependency issues.

## What Moves

### Validator: `nullslop-intent/src/validators/chat_entry.rs` (partial) → `nsslice-chat-entry-selection/src/validator.rs`

**Move these** (top of the file):

- 2 infallible validators: `validate_chat_entry_select_next`, `validate_chat_entry_select_prev`
- 1 fallible validator: `validate_chat_entry_pin_selected` → `ChatEntryPinSelectedError`
- 1 error enum: `ChatEntryPinSelectedError`
- 3 validator tests: `pin_selected_succeeds_with_selected_entry`, `pin_selected_fails_with_empty_history`, `pin_selected_fails_with_no_selection`

**Keep in `nullslop-intent/src/validators/chat_entry.rs`** (bottom of the file):

- `validate_refresh_models` + `RefreshModelsError`
- `validate_rescan_prompt_templates` + `RescanPromptTemplatesError`
- `validate_session_new` + `SessionNewError`
- 4 validator tests: `refresh_models_succeeds_with_provider`, `refresh_models_fails_with_no_provider`, `session_new_succeeds_when_no_picker_active`, `session_new_fails_when_picker_active`

The kept validators have **no dependency** on the moved ones — they use different `AppState` fields entirely (`active_provider`, `active_picker_kind`).

### Handler: `nullslop-intent/src/handler.rs` → `nsslice-chat-entry-selection/src/intent.rs`

**Move 3 match arms** (from the `// --- Chat Entry Selection ---` section):

1. `Intent::ChatEntrySelectNext` — inline validate + `select_next_entry()` → becomes `handle_select_next(state)`
2. `Intent::ChatEntrySelectPrev` — inline validate + `select_prev_entry()` → becomes `handle_select_prev(state)`
3. `Intent::ChatEntryPinSelected` — calls `handle_chat_entry_pin_selected` helper → becomes `handle_pin_selected(state)`

**Move 1 helper function**:

- `handle_chat_entry_pin_selected(state: &mut AppState) -> IntentResult` — becomes `handle_pin_selected` (public)

### Tests: `nullslop-intent/src/handler_tests.rs` → `nsslice-chat-entry-selection/src/intent.rs`

**Move 4 tests** (from the `// ============ Chat Entry Selection Intents ============` section):

1. `chat_entry_select_next_increments_index`
2. `chat_entry_select_prev_decrements_index`
3. `chat_entry_pin_selected_returns_pin_command`
4. `chat_entry_pin_selected_noop_with_no_selection`

### What stays in `nullslop-intent`

- `SessionNew`, `RefreshModels`, `RescanPromptTemplates` match arms + handler functions — use `chat_entry::*` validators that stay behind
- `handle_interrupt`, `handle_set_mode`, `handle_normal_escape` — cross-cutting handlers
- All navigation intents (scroll, tab, edit)
- `cancel_stream_and_drain` helper

## File Changes

### 1. NEW `crates/slices/nsslice-chat-entry-selection/Cargo.toml`

```toml
[package]
name = "nsslice-chat-entry-selection"
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

- `nullslop-component` — for `AppState`, session state methods
- `nullslop-protocol` — for `IntentResult`, `Command`, `PinChatEntry`, `PinPosition`
- `wherror` — for `#[derive(Error)]` on `ChatEntryPinSelectedError`
- `rstest` — test framework (dev-dep)

Note: No `jiff` dev-dep needed — `ChatEntry::user()` doesn't require timestamps.

### 2. NEW `crates/slices/nsslice-chat-entry-selection/src/lib.rs`

```rust
//! Chat entry selection slice — navigate and pin chat log entries.
//!
//! Handles selecting the next/previous chat entry and pinning the
//! selected entry. No element — rendering stays in `nullslop-tui`.

pub mod intent;
pub mod validator;
```

### 3. NEW `crates/slices/nsslice-chat-entry-selection/src/validator.rs`

Move the top portion of `nullslop-intent/src/validators/chat_entry.rs`:

- `validate_chat_entry_select_next`
- `validate_chat_entry_select_prev`
- `ChatEntryPinSelectedError` enum
- `validate_chat_entry_pin_selected`
- 3 tests

Imports change from `use nullslop_component::AppState;` (same — already absolute).

In the `#[cfg(test)]` block, the test imports change:
- Old: `use crate::validators::picker::{...}` style — but `chat_entry.rs` tests just use `super::*`, so no change needed.

### 4. NEW `crates/slices/nsslice-chat-entry-selection/src/intent.rs`

Contains 3 public handler functions and 4 handler tests:

```rust
//! Chat entry selection intent handlers — navigate and pin entries.

use nullslop_component::AppState;
use nullslop_protocol::context::PinChatEntry;
use nullslop_protocol::{Command, IntentResult, PinPosition};

use crate::validator;

/// Selects the next chat entry in the active session.
pub fn handle_select_next(state: &mut AppState) -> IntentResult {
    validator::validate_chat_entry_select_next(state);
    state.active_session_mut().select_next_entry();
    IntentResult::empty()
}

/// Selects the previous chat entry in the active session.
pub fn handle_select_prev(state: &mut AppState) -> IntentResult {
    validator::validate_chat_entry_select_prev(state);
    state.active_session_mut().select_prev_entry();
    IntentResult::empty()
}

/// Pins the currently selected chat entry.
///
/// Returns a `PinChatEntry` command with `Relative` position.
pub fn handle_pin_selected(state: &mut AppState) -> IntentResult {
    if validator::validate_chat_entry_pin_selected(state).is_err() {
        return IntentResult::empty();
    }

    let session_id = state.session.active_session.clone();
    let Some(entry_id) = state.active_session().selected_entry_id().cloned() else {
        return IntentResult::empty();
    };

    IntentResult::with_commands(vec![Command::PinChatEntry {
        payload: PinChatEntry {
            session_id,
            entry_id,
            position: PinPosition::Relative,
        },
    }])
}
```

Tests will call handler functions directly (not through `IntentHandler::handle`).

### 5. Root `Cargo.toml`

**members line** — add `"crates/slices/nsslice-chat-entry-selection"`:

```
members = [..., "crates/slices/nsslice-picker", "crates/slices/nsslice-chat-entry-selection", "actors/*", "tests/*"]
```

**[workspace.dependencies]** — add:
```toml
nsslice-chat-entry-selection = { path = "crates/slices/nsslice-chat-entry-selection" }
```

**[dependencies]** — add:
```toml
nsslice-chat-entry-selection = { workspace = true }
```

### 6. `crates/nullslop-intent/Cargo.toml`

Add:
```toml
nsslice-chat-entry-selection = { workspace = true }
```

### 7. `crates/nullslop-intent/src/handler.rs`

**Replace 3 chat entry match arms**:

```rust
// --- Chat Entry Selection ---
Intent::ChatEntrySelectNext => {
    nsslice_chat_entry_selection::intent::handle_select_next(state)
}
Intent::ChatEntrySelectPrev => {
    nsslice_chat_entry_selection::intent::handle_select_prev(state)
}
Intent::ChatEntryPinSelected => {
    nsslice_chat_entry_selection::intent::handle_pin_selected(state)
}
```

**Remove helper function**: `handle_chat_entry_pin_selected` — the entire function and its `// --- Chat Entry handlers ---` comment section.

**Clean up imports**: After removing `handle_chat_entry_pin_selected`, the following imports are no longer needed in `handler.rs`:
- `nullslop_protocol::context::PinChatEntry` — only used in the removed function
- `nullslop_protocol::PinPosition` — only used in the removed function

The import line changes from:
```rust
use nullslop_protocol::context::PinChatEntry;
use nullslop_protocol::provider::CancelStream;
use nullslop_protocol::{Command, Mode, PinPosition, SessionId, TabDirection};
```
to:
```rust
use nullslop_protocol::provider::CancelStream;
use nullslop_protocol::{Command, Mode, SessionId, TabDirection};
```

**Keep using `chat_entry::*`** for `SessionNew`, `RefreshModels`, `RescanPromptTemplates` validators — those stay.

### 8. `crates/nullslop-intent/src/validators/chat_entry.rs` (partial update)

Remove the moved items from the top:
- Remove `validate_chat_entry_select_next`
- Remove `validate_chat_entry_select_prev`
- Remove `ChatEntryPinSelectedError` enum
- Remove `validate_chat_entry_pin_selected`
- Remove the "Infallible validators" section comment and the "ChatEntryPinSelected" tests from the test module

Keep everything else (the three remaining validators, their error enums, and 4 tests).

Update the module-level doc comment from "Chat entry selection and related intent validators" to something like "Session and model intent validators" since the selection validators have moved out.

### 9. `crates/nullslop-intent/src/handler_tests.rs`

**Remove 4 chat entry selection tests** (the `// ============ Chat Entry Selection Intents ============` section + the 4 tests below it).

After removal, the imports can be cleaned up:
- `PinChatEntry` import — no longer used (was only used in `chat_entry_pin_selected_returns_pin_command`)
- `ChatEntry` import — still used by `session_new_creates_fresh_session`
- `PinPosition` import — no longer used

Import changes:
```rust
// Old:
use nullslop_protocol::context::PinChatEntry;
use nullslop_protocol::tab::TabDirection;
use nullslop_protocol::{ChatEntry, Command, Mode, PickerKind, PinPosition};

// New:
use nullslop_protocol::tab::TabDirection;
use nullslop_protocol::{ChatEntry, Command, Mode, PickerKind};
```

## Implementation Order

1. Create `crates/slices/nsslice-chat-entry-selection/Cargo.toml`
2. Create `crates/slices/nsslice-chat-entry-selection/src/lib.rs`
3. Create `crates/slices/nsslice-chat-entry-selection/src/validator.rs` — move 3 validators + error enum + 3 tests
4. Create `crates/slices/nsslice-chat-entry-selection/src/intent.rs` — move 3 handler functions + 4 tests
5. Add `nsslice-chat-entry-selection` to root `Cargo.toml` (members, workspace.dependencies, dependencies)
6. Add `nsslice-chat-entry-selection` dep to `nullslop-intent/Cargo.toml`
7. Update `nullslop-intent/src/handler.rs` — replace 3 match arms, remove helper function, clean imports
8. Update `nullslop-intent/src/validators/chat_entry.rs` — remove moved validators/tests, update doc comment
9. Update `nullslop-intent/src/handler_tests.rs` — remove 4 tests, clean imports
10. Run `cargo test --workspace`

## Acceptance Criteria

1. `crates/slices/nsslice-chat-entry-selection/` exists with `Cargo.toml`, `src/lib.rs`, `src/validator.rs`, `src/intent.rs`
2. `nsslice-chat-entry-selection` is a workspace member in root `Cargo.toml`
3. `nullslop-intent/Cargo.toml` has `nsslice-chat-entry-selection` dependency
4. `nsslice-chat-entry-selection` has 3 handler tests + 3 validator tests, all passing independently (`cargo test -p nsslice-chat-entry-selection`)
5. `nullslop-intent/src/validators/chat_entry.rs` still has `RefreshModels`, `RescanPromptTemplates`, `SessionNew` validators + 4 tests
6. `nullslop-intent/src/handler.rs` has 3 chat entry match arms delegating to `nsslice_chat_entry_selection::intent::*`
7. `nullslop-intent/src/handler.rs` no longer has `handle_chat_entry_pin_selected` helper or `PinChatEntry`/`PinPosition` imports
8. `nullslop-intent/src/handler_tests.rs` no longer has chat entry selection tests
9. `cargo test --workspace` passes — no regressions
