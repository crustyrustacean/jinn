# Phase 5: Extract `nsslice-pinned-panel` (First Intent-Bearing Slice)

This phase extracts the pinned panel feature — element, validators, and intent
handlers — into a new slice crate at `crates/slices/nsslice-pinned-panel/`. This is
the first intent-bearing slice and establishes the pattern for Phase 6 (chat-input-box).

## Context

The pinned panel feature currently spans 3 locations:
- **Element**: `nullslop-component/src/pinned_panel/element.rs` — rendering
- **State**: `nullslop-component/src/pinned_panel/state.rs` — stays in place
- **Validators**: `nullslop-intent/src/validators/pinned_panel.rs` — 5 validators + error type
- **Handlers**: `nullslop-intent/src/handler.rs` — 3 handler functions + 2 helpers + 6 inline match arms

The element uses `pin_sort_key` from `nullslop_component::app_state` (public module).
The intent handlers use `resolve_selected_entry_id` which calls
`nullslop_component::app_state::pin_sort_key` as well.

### Dependency chain

The slice crate depends on:
- `nullslop-component` — for `AppState`, `AppUiRegistry`, `app_state::pin_sort_key`
- `nullslop-component-ui` — for `UiElement` trait
- `nullslop-protocol` — for `IntentResult`, `Command`, `PinPosition`, `ChatEntry`, `SessionId`, `UnpinChatEntry`, `PinChatEntry`
- `ratatui` — for element rendering
- `wherror` — for validator error type (`PinnedPanelActionError`)

`nullslop-intent` gains a dependency on `nsslice-pinned-panel` so its handler match
arms can call `nsslice_pinned_panel::intent::*()`.

## Detailed Steps

### 1. Create `crates/slices/nsslice-pinned-panel/Cargo.toml`

```toml
[package]
name = "nsslice-pinned-panel"
version = "0.1.0"
edition = "2024"

[dependencies]
nullslop-component-ui = { workspace = true }
nullslop-component = { workspace = true }
nullslop-protocol = { workspace = true }
ratatui = { workspace = true }
wherror = { workspace = true }

[dev-dependencies]
rstest = { workspace = true }

[lints]
workspace = true
```

### 2. Create `crates/slices/nsslice-pinned-panel/src/lib.rs`

```rust
//! Pinned panel slice — context entry pinning UI, validation, and intent handling.
//!
//! Co-locates everything about the pinned context panel:
//!
//! - **Element** — renders pinned entries with position badges and selection highlighting.
//! - **Validator** — validates pin/unpin actions (checks selection, checks entries exist).
//! - **Intent** — handles all 11 pinned-panel intents (toggle, open, close, select,
//!   unpin, pin top/bottom/relative, pin cycle).
//!
//! State (`PinnedPanelState`) stays in `nullslop-component` to avoid circular dependencies.

pub mod element;
pub mod intent;
pub mod validator;

pub use element::PinnedPanelElement;

use nullslop_component::AppUiRegistry;

/// Register pinned panel UI element.
pub fn register(registry: &mut AppUiRegistry) {
    registry.register(Box::new(PinnedPanelElement));
}
```

### 3. Create `crates/slices/nsslice-pinned-panel/src/element.rs`

Copy from `nullslop-component/src/pinned_panel/element.rs` with import changes:

```rust
// Before:
use crate::AppState;
use crate::app_state::pin_sort_key;

// After:
use nullslop_component::AppState;
use nullslop_component::app_state::pin_sort_key;
```

All other imports (`nullslop_component_ui::UiElement`, `nullslop_protocol::*`, `ratatui::*`)
remain unchanged — they were already absolute.

Move all tests with the element (7 tests: `name_returns_pinned_panel`,
`render_no_entries_shows_message`, `render_shows_pinned_entries`,
`render_selected_entry_has_yellow_marker`, `pinned_panel_element_is_selectable`,
`render_sorts_entries_by_position`).

Update test imports:
```rust
// Before:
use crate::AppState;

// After:
use nullslop_component::AppState;
```

### 4. Create `crates/slices/nsslice-pinned-panel/src/validator.rs`

Copy from `nullslop-intent/src/validators/pinned_panel.rs` with import changes:

```rust
// Before:
use nullslop_component::AppState;

// After: (no change — already uses nullslop_component::AppState)
```

The validator file already imports `nullslop_component::AppState` — no import changes needed.
Move all validator tests (3 tests in `unpin_succeeds_with_selected_pinned_entry`,
`unpin_fails_with_no_pinned_entries`, `unpin_fails_with_no_selection`).

### 5. Create `crates/slices/nsslice-pinned-panel/src/intent.rs`

This file contains the handler functions and their tests. Move from `handler.rs`:

**Handler functions:**
- `handle_pinned_panel_unpin(state: &mut AppState) -> IntentResult`
- `handle_pinned_panel_pin(state: &mut AppState, position: PinPosition) -> IntentResult`
- `handle_pinned_panel_pin_cycle(state: &mut AppState) -> IntentResult`

**New thin wrappers for the 5 inline match arms:**
- `handle_toggle(state: &mut AppState) -> IntentResult` — sets `pinned_pane_toggle` signal
- `handle_open(state: &mut AppState) -> IntentResult` — sets `pinned_pane_open` signal
- `handle_close(state: &mut AppState) -> IntentResult` — sets `pinned_pane_close` signal
- `handle_select_down(state: &mut AppState) -> IntentResult` — calls `select_next`
- `handle_select_up(state: &mut AppState) -> IntentResult` — calls `select_prev`

**Helper functions:**
- `resolve_selected_entry_id(state: &AppState) -> Option<(SessionId, ChatEntryId)>`
- `cycle_position(pos: PinPosition) -> PinPosition`

**Imports:**
```rust
use nullslop_component::AppState;
use nullslop_component::app_state::pin_sort_key;
use nullslop_protocol::{
    ChatEntryId, Command, IntentResult, PinPosition, SessionId,
};
use nullslop_protocol::context::{PinChatEntry, UnpinChatEntry};

use crate::validator;
```

**Tests:** Move all 14 pinned-panel tests from `nullslop-intent/src/handler_tests.rs`:
- `pinned_panel_toggle_sets_signal`
- `pinned_panel_open_sets_signal`
- `pinned_panel_close_sets_signal`
- `pinned_panel_select_down_moves_selection`
- `pinned_panel_select_up_moves_selection`
- `pinned_panel_unpin_returns_command`
- `pinned_panel_unpin_noop_when_empty`
- `pinned_panel_pin_top_returns_command`
- `pinned_panel_pin_bottom_returns_command`
- `pinned_panel_pin_relative_returns_command`
- `pinned_panel_pin_cycle_rotates_top_to_bottom`
- `pinned_panel_pin_cycle_noop_when_empty`
- `pinned_panel_pin_top_noop_when_no_selection`

Also move the shared helper `fn state_with_pinned(count: usize) -> AppState`.

**Important**: The tests currently call `handle(&Intent::PinnedPanelToggle, &mut state)`
which delegates to `IntentHandler::handle()`. After moving to the slice, tests should
call the slice's own handler functions directly (e.g., `super::handle_toggle(&mut state)`)
since we're testing slice logic, not the central dispatch. For the handler functions
that take `Intent` as input (`handle_toggle`, etc.), call them directly. For tests
that assert on `Command` results, call the specific handler function.

The test helper changes from:
```rust
fn handle(intent: &Intent, state: &mut AppState) -> super::IntentResult {
    IntentHandler::handle(intent, state)
}
```
To direct calls to the slice functions. Each test calls the specific handler function
directly instead of going through `handle(&Intent::...)`.

### 6. Delete `nullslop-component/src/pinned_panel/element.rs`

After copying to the slice, delete the element file.

### 7. Update `nullslop-component/src/pinned_panel/mod.rs`

Remove `pub mod element;` and `pub use element::PinnedPanelElement;`. Keep only:
```rust
pub mod state;
pub use state::PinnedPanelState;
```

### 8. Remove `PinnedPanelElement` from `register_tui_elements()`

In `nullslop-component/src/lib.rs`, remove the line:
```rust
registry.register(Box::new(pinned_panel::PinnedPanelElement));
```

### 9. Update `nullslop-intent/src/handler.rs`

**Remove:**
- `use crate::validators::pinned_panel;` from imports
- The entire `// --- Pinned Panel handlers ---` section: `resolve_selected_entry_id`,
  `cycle_position`, `handle_pinned_panel_unpin`, `handle_pinned_panel_pin`,
  `handle_pinned_panel_pin_cycle`
- The `pin_sort_key` import (only used by removed handlers, but verify)

**Update match arms** to call slice functions:

```rust
// --- Pinned Panel ---
Intent::PinnedPanelToggle => {
    nsslice_pinned_panel::intent::handle_toggle(state)
}
Intent::PinnedPanelOpen => {
    nsslice_pinned_panel::intent::handle_open(state)
}
Intent::PinnedPanelClose => {
    nsslice_pinned_panel::intent::handle_close(state)
}
Intent::PinnedPanelSelectDown => {
    nsslice_pinned_panel::intent::handle_select_down(state)
}
Intent::PinnedPanelSelectUp => {
    nsslice_pinned_panel::intent::handle_select_up(state)
}
Intent::PinnedPanelUnpin => {
    nsslice_pinned_panel::intent::handle_pinned_panel_unpin(state)
}
Intent::PinnedPanelPinTop => {
    nsslice_pinned_panel::intent::handle_pinned_panel_pin(state, PinPosition::Top)
}
Intent::PinnedPanelPinBottom => {
    nsslice_pinned_panel::intent::handle_pinned_panel_pin(state, PinPosition::Bottom)
}
Intent::PinnedPanelPinRelative => {
    nsslice_pinned_panel::intent::handle_pinned_panel_pin(state, PinPosition::Relative)
}
Intent::PinnedPanelPinCycle => {
    nsslice_pinned_panel::intent::handle_pinned_panel_pin_cycle(state)
}
```

**Keep `NormalEscape` in `nullslop-intent`** — it's cross-cutting (clears chat entry
selection + sets pinned_pane_close signal). The `pinned_pane_close` line stays.

### 10. Delete `nullslop-intent/src/validators/pinned_panel.rs`

### 11. Update `nullslop-intent/src/validators/mod.rs`

Remove `pub mod pinned_panel;`

### 12. Move pinned-panel tests from `handler_tests.rs`

Remove all pinned-panel test functions and the `state_with_pinned` helper from
`nullslop-intent/src/handler_tests.rs`. These are the 14 test functions listed
in step 5, from the `// Pinned Panel Intents` section header through the last
pinned-panel test (`pinned_panel_pin_top_noop_when_no_selection`).

### 13. Add workspace entries in root `Cargo.toml`

**`[workspace] members`** — add `"crates/slices/nsslice-pinned-panel"` to the list.

**`[workspace.dependencies]`** — add:
```toml
nsslice-pinned-panel = { path = "crates/slices/nsslice-pinned-panel" }
```

**`[dependencies]`** (root package) — add:
```toml
nsslice-pinned-panel = { workspace = true }
```

### 14. Wire registration in `src/app.rs`

Add `nsslice_pinned_panel::register(&mut ui_registry)` at both registration sites,
after the existing slice registrations:

**TUI path** (~line 143):
```rust
nsslice_provider::register(&mut ui_registry);
nsslice_pinned_panel::register(&mut ui_registry);  // ADD
```

**Headless path** (~line 514):
```rust
nsslice_provider::register(&mut registry);
nsslice_pinned_panel::register(&mut registry);  // ADD
```

### 15. Add `nsslice-provider` dependency to `crates/nullslop-tui/Cargo.toml`

Add `nsslice-pinned-panel = { workspace = true }` to dependencies.

### 16. Add registration in `crates/nullslop-tui/src/app.rs`

Add `nsslice_pinned_panel::register(&mut ui_registry)` at all 3 registration sites,
after the existing slice registrations (after `nsslice_provider::register()`).

### 17. Add `nsslice-pinned-panel` dependency to `crates/nullslop-intent/Cargo.toml`

Add `nsslice-pinned-panel = { workspace = true }` to dependencies.

### 18. Verify

```bash
cargo test --workspace
cargo test -p nsslice-pinned-panel
cargo test -p nullslop-intent
```

## Acceptance Criteria

1. **No duplicated code**: `nullslop-component/src/pinned_panel/element.rs` and
   `nullslop-intent/src/validators/pinned_panel.rs` are deleted. Handler functions
   removed from `nullslop-intent/src/handler.rs`.
2. **No orphaned registrations**: `register_tui_elements()` in `nullslop-component/src/lib.rs`
   does not register `PinnedPanelElement`.
3. **Slice registered everywhere**: `nsslice_pinned_panel::register()` called at all 5 sites
   (2 in `src/app.rs`, 3 in `crates/nullslop-tui/src/app.rs`).
4. **Intent dispatch wired**: `nullslop-intent/src/handler.rs` calls into
   `nsslice_pinned_panel::intent::*()` for all 11 pinned-panel intents.
5. **All tests moved**: 14 pinned-panel handler tests + 3 validator tests + 7 element tests
   run in `nsslice-pinned-panel`. None remain in `nullslop-intent`.
6. **State stays**: `PinnedPanelState` remains in `nullslop-component/src/pinned_panel/state.rs`.
7. **No circular dependencies**: `nsslice-pinned-panel` depends on `nullslop-component`, not vice versa.
   `nullslop-intent` depends on `nsslice-pinned-panel`, not vice versa.
8. **Full workspace tests pass**: `cargo test --workspace` passes with no failures.
9. **Independently testable**: `cargo test -p nsslice-pinned-panel` passes.

## Files Changed

### Created
- `crates/slices/nsslice-pinned-panel/Cargo.toml`
- `crates/slices/nsslice-pinned-panel/src/lib.rs`
- `crates/slices/nsslice-pinned-panel/src/element.rs`
- `crates/slices/nsslice-pinned-panel/src/validator.rs`
- `crates/slices/nsslice-pinned-panel/src/intent.rs`

### Deleted
- `crates/nullslop-component/src/pinned_panel/element.rs`
- `crates/nullslop-intent/src/validators/pinned_panel.rs`

### Modified
- `Cargo.toml` (root) — workspace members + dependencies
- `crates/nullslop-component/src/pinned_panel/mod.rs` — remove element submodule + re-export
- `crates/nullslop-component/src/lib.rs` — remove `PinnedPanelElement` registration
- `crates/nullslop-intent/src/handler.rs` — remove handlers, delegate to slice
- `crates/nullslop-intent/src/handler_tests.rs` — remove pinned-panel tests
- `crates/nullslop-intent/src/validators/mod.rs` — remove `pinned_panel` module
- `crates/nullslop-intent/Cargo.toml` — add `nsslice-pinned-panel` dependency
- `crates/nullslop-tui/Cargo.toml` — add `nsslice-pinned-panel` dependency
- `crates/nullslop-tui/src/app.rs` — add registration at 3 sites
- `src/app.rs` — add registration at 2 sites
