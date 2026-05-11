# Phase 1: Extract Navigation → `nsslice-navigation`

## Problem

Eight navigation intents (`ScrollUp`, `ScrollDown`, `MouseScrollUp`, `MouseScrollDown`, `ScrollToTop`, `ScrollToBottom`, `SwitchTab`, `EditInput`) are still inline in the `nullslop-intent` match block. These are the simplest handlers in the codebase — pure state mutation, no validators, no commands, no cross-cutting concerns. Every one just mutates `active_session_mut()` or `frontend` and returns `IntentResult::empty()`.

## What Moves

### From `nullslop-intent/src/handler.rs`:

**Constants:**
- `SCROLL_STEP: u16 = 10` (associated const on `IntentHandler`)
- `MOUSE_SCROLL_STEP: u16 = 3` (associated const on `IntentHandler`)

**8 inline match arms** (lines ~147–188 in the `// --- Navigation ---` section):

1. `Intent::ScrollUp` → `handle_scroll_up(state)`
2. `Intent::ScrollDown` → `handle_scroll_down(state)`
3. `Intent::MouseScrollUp` → `handle_mouse_scroll_up(state)`
4. `Intent::MouseScrollDown` → `handle_mouse_scroll_down(state)`
5. `Intent::ScrollToTop` → `handle_scroll_to_top(state)`
6. `Intent::ScrollToBottom` → `handle_scroll_to_bottom(state)`
7. `Intent::SwitchTab { direction }` → `handle_switch_tab(state, *direction)`
8. `Intent::EditInput` → `handle_edit_input(state)`

**No helper functions** to move — all logic is inline.
**No validators** to move — none exist.

### From `nullslop-intent/src/handler_tests.rs`:

**8 tests** (the `// ============ Navigation Intents ============` section):

1. `scroll_up_decrements_scroll_offset`
2. `scroll_down_increments_scroll_offset`
3. `mouse_scroll_up_decrements_scroll_offset`
4. `mouse_scroll_down_increments_scroll_offset`
5. `scroll_to_top_sets_offset_to_zero`
6. `scroll_to_bottom_resets_scroll`
7. `switch_tab_next_advances_tab`
8. `edit_input_sets_tui_signal`

### What stays in `nullslop-intent`

Everything else. No changes to other match arms, helper functions, or validators. The only changes are removing the 8 Navigation match arms and 8 tests.

After this phase, `handler.rs` will have `TabDirection` removed from imports (only used in the removed `SwitchTab` arm).

## File Changes

### 1. NEW `crates/slices/nsslice-navigation/Cargo.toml`

```toml
[package]
name = "nsslice-navigation"
version = "0.1.0"
edition = "2024"

[dependencies]
nullslop-component = { workspace = true }
nullslop-protocol = { workspace = true }

[dev-dependencies]
rstest = { workspace = true }

[lints]
workspace = true
```

- `nullslop-component` — for `AppState`, session methods, `FrontendState`
- `nullslop-protocol` — for `IntentResult`, `TabDirection`, `ActiveTab`
- No `wherror` needed — no fallible validators
- `rstest` — test framework (dev-dep)

### 2. NEW `crates/slices/nsslice-navigation/src/lib.rs`

```rust
//! Navigation slice — scroll, tab switching, and external editor.
//!
//! Handles scrolling the chat log, switching between tabs,
//! and opening the external editor. No element — rendering
//! stays in `nullslop-tui`.

pub mod intent;
```

No `validator` module — all 8 intents are infallible. No element — no `register()`.

### 3. NEW `crates/slices/nsslice-navigation/src/intent.rs`

Contains 2 constants, 8 public handler functions, and 8 tests:

```rust
//! Navigation intent handlers — scroll, tab, and editor.

use nullslop_component::AppState;
use nullslop_protocol::tab::TabDirection;
use nullslop_protocol::IntentResult;

/// Number of lines to scroll per keyboard step.
const SCROLL_STEP: u16 = 10;
/// Number of lines to scroll per mouse wheel tick.
const MOUSE_SCROLL_STEP: u16 = 3;

/// Scrolls the chat log up by one keyboard step.
pub fn handle_scroll_up(state: &mut AppState) -> IntentResult {
    state.active_session_mut().scroll_up(SCROLL_STEP);
    IntentResult::empty()
}

/// Scrolls the chat log down by one keyboard step.
pub fn handle_scroll_down(state: &mut AppState) -> IntentResult {
    state.active_session_mut().scroll_down(SCROLL_STEP);
    IntentResult::empty()
}

/// Scrolls the chat log up by one mouse wheel tick.
pub fn handle_mouse_scroll_up(state: &mut AppState) -> IntentResult {
    state.active_session_mut().scroll_up(MOUSE_SCROLL_STEP);
    IntentResult::empty()
}

/// Scrolls the chat log down by one mouse wheel tick.
pub fn handle_mouse_scroll_down(state: &mut AppState) -> IntentResult {
    state.active_session_mut().scroll_down(MOUSE_SCROLL_STEP);
    IntentResult::empty()
}

/// Scrolls the chat log to the very top.
pub fn handle_scroll_to_top(state: &mut AppState) -> IntentResult {
    state.active_session_mut().scroll_to_top();
    IntentResult::empty()
}

/// Scrolls the chat log to the very bottom.
pub fn handle_scroll_to_bottom(state: &mut AppState) -> IntentResult {
    state.active_session_mut().scroll_to_bottom();
    IntentResult::empty()
}

/// Switches to the next or previous tab.
pub fn handle_switch_tab(state: &mut AppState, direction: TabDirection) -> IntentResult {
    state.frontend.active_tab = match direction {
        TabDirection::Next => state.frontend.active_tab.next(),
        TabDirection::Prev => state.frontend.active_tab.prev(),
    };
    IntentResult::empty()
}

/// Opens the input in an external editor.
pub fn handle_edit_input(state: &mut AppState) -> IntentResult {
    state.frontend.tui_signals.edit_requested = true;
    IntentResult::empty()
}
```

Tests move from `handler_tests.rs`. In the new crate, tests call handler functions directly (not through `IntentHandler::handle`). Test imports:

```rust
#[cfg(test)]
mod tests {
    use nullslop_component::AppState;
    use nullslop_protocol::tab::TabDirection;
    use nullslop_protocol::ChatEntry;

    use super::*;
    // ...
}
```

### 4. Root `Cargo.toml`

**members line** — add `"crates/slices/nsslice-navigation"`:

```
members = [..., "crates/slices/nsslice-chat-entry-selection", "crates/slices/nsslice-navigation", "actors/*", "tests/*"]
```

**[workspace.dependencies]** — add:
```toml
nsslice-navigation = { path = "crates/slices/nsslice-navigation" }
```

**[dependencies]** — add:
```toml
nsslice-navigation = { workspace = true }
```

### 5. `crates/nullslop-intent/Cargo.toml`

Add:
```toml
nsslice-navigation = { workspace = true }
```

### 6. `crates/nullslop-intent/src/handler.rs`

**Replace 8 inline Navigation match arms** with delegations:

```rust
// --- Navigation ---
Intent::ScrollUp => nsslice_navigation::intent::handle_scroll_up(state),
Intent::ScrollDown => nsslice_navigation::intent::handle_scroll_down(state),
Intent::MouseScrollUp => nsslice_navigation::intent::handle_mouse_scroll_up(state),
Intent::MouseScrollDown => nsslice_navigation::intent::handle_mouse_scroll_down(state),
Intent::ScrollToTop => nsslice_navigation::intent::handle_scroll_to_top(state),
Intent::ScrollToBottom => nsslice_navigation::intent::handle_scroll_to_bottom(state),
Intent::SwitchTab { direction } => {
    nsslice_navigation::intent::handle_switch_tab(state, *direction)
}
Intent::EditInput => nsslice_navigation::intent::handle_edit_input(state),
```

**Remove constants**: `SCROLL_STEP` and `MOUSE_SCROLL_STEP` from `IntentHandler` impl.

**Clean up imports**: Remove `TabDirection` from the import line (no longer used in handler.rs — only used in the removed SwitchTab arm):

```rust
// Old:
use nullslop_protocol::{Command, Mode, PinPosition, SessionId, TabDirection};
// New:
use nullslop_protocol::{Command, Mode, PinPosition, SessionId};
```

### 7. `crates/nullslop-intent/src/handler_tests.rs`

**Remove 8 navigation tests** (the entire `// ============ Navigation Intents ============` section + all 8 tests below it).

**Clean up imports**: After removing navigation tests:
- `TabDirection` — no longer used → remove from imports
- `ChatEntry` — still used by `normal_escape_clears_selection`, `session_new_creates_fresh_session` → keep
- `Command` — still used by interrupt and set_mode tests → keep

```rust
// Old:
use nullslop_protocol::tab::TabDirection;
use nullslop_protocol::{ChatEntry, Command, Mode, PickerKind};
// New:
use nullslop_protocol::{ChatEntry, Command, Mode, PickerKind};
```

## Implementation Order

1. Create `crates/slices/nsslice-navigation/Cargo.toml`
2. Create `crates/slices/nsslice-navigation/src/lib.rs`
3. Create `crates/slices/nsslice-navigation/src/intent.rs` — 8 handlers + 8 tests
4. Add `nsslice-navigation` to root `Cargo.toml` (members, workspace.dependencies, dependencies)
5. Add `nsslice-navigation` dep to `nullslop-intent/Cargo.toml`
6. Update `nullslop-intent/src/handler.rs` — replace 8 match arms, remove constants, clean imports
7. Update `nullslop-intent/src/handler_tests.rs` — remove 8 tests, clean imports
8. Run `cargo test --workspace`

## Acceptance Criteria

1. `crates/slices/nsslice-navigation/` exists with `Cargo.toml`, `src/lib.rs`, `src/intent.rs`
2. `nsslice-navigation` is a workspace member in root `Cargo.toml`
3. `nullslop-intent/Cargo.toml` has `nsslice-navigation` dependency
4. `nsslice-navigation` has 8 handler tests, all passing independently (`cargo test -p nsslice-navigation`)
5. `nullslop-intent/src/handler.rs` has 8 navigation match arms delegating to `nsslice_navigation::intent::*`
6. `nullslop-intent/src/handler.rs` no longer has `SCROLL_STEP`, `MOUSE_SCROLL_STEP`, or `TabDirection`
7. `nullslop-intent/src/handler_tests.rs` no longer has navigation tests or `TabDirection` import
8. `cargo test --workspace` passes — no regressions
