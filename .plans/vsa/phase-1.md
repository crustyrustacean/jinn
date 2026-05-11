# Phase 1: Cleanup Orphaned Code + Extract `nsslice-chat-log`

This phase merges the original Phase 1 (cleanup) and Phase 2 (extract chat-log) from the
high-level plan. Both are display-only extractions following the exact same pattern as the
3 already-extracted slices (`nsslice-char-counter`, `nsslice-dashboard`, `nsslice-status-bar`).

## Context

Three slices were already extracted to `crates/slices/`, but the old element files still exist
in `nullslop-component/src/{char_counter,status_bar,dashboard}/element.rs`. These are dead code —
not declared in `nullslop-component/src/lib.rs` and not registered in `register_tui_elements()`.
They should be removed to eliminate confusion about which file is the source of truth.

Meanwhile, `chat_log` is the largest display-only component (833 lines) still in
`nullslop-component`. It follows the same extraction pattern. Extracting it alongside the cleanup
is natural since both tasks touch the same files (`nullslop-component/src/lib.rs`, `src/app.rs`,
root `Cargo.toml`).

## Slice Pattern (established by existing slices)

Every slice crate under `crates/slices/` follows this structure:

```
nsslice-<feature>/
├── Cargo.toml
│   edition 2024
│   depends on: nullslop-component-ui, nullslop-component, ratatui
│   dev-dep: rstest
│   lints: workspace = true
├── src/
│   ├── lib.rs      — pub mod element; pub use element::*; register(&mut AppUiRegistry)
│   └── element.rs  — UiElement<AppState> impl + #[cfg(test)] mod tests
```

Imports use `nullslop_component::AppState` instead of `crate::AppState`.

Registration is wired in two places in `src/app.rs`:
- TUI path (~line 139): after `nullslop_component::register_tui_elements(&mut ui_registry)`
- Headless path (~line 507): after `nullslop_component::register_all(&mut registry)`

Workspace entries go in root `Cargo.toml`:
- `[workspace] members` list (explicit, since `crates/slices` is excluded from the glob)
- `[workspace.dependencies]` with path

## Detailed Steps

### Part A: Cleanup orphaned code

#### 1. Delete `nullslop-component/src/char_counter/` directory entirely

This directory contains `mod.rs` (6 lines, just declares `element` submodule and re-exports
`CharCounterElement`) and `element.rs` (130 lines, the duplicate element impl). Both are dead code
since `char_counter` is not declared in `nullslop-component/src/lib.rs`. The element was moved to
`nsslice-char-counter`.

**Action**: `rm -rf crates/nullslop-component/src/char_counter/`

No changes needed in `lib.rs` — `char_counter` is already not declared there.

#### 2. Delete `nullslop-component/src/status_bar/` directory entirely

Same situation: `mod.rs` (6 lines) + `element.rs` (359 lines). Not declared in `lib.rs`.
Moved to `nsslice-status-bar`.

**Action**: `rm -rf crates/nullslop-component/src/status_bar/`

No changes needed in `lib.rs`.

#### 3. Delete `nullslop-component/src/dashboard/element.rs`

The `dashboard` module is still declared in `lib.rs` because it exports `DashboardState`
(from `state.rs`). Only the `element.rs` file on disk is dead code — the current
`dashboard/mod.rs` already only declares `pub mod state;` and re-exports `DashboardState`.
The element was moved to `nsslice-dashboard`.

**Action**: Delete `crates/nullslop-component/src/dashboard/element.rs`.
No changes to `dashboard/mod.rs` needed.

#### 4. Verify `register_tui_elements()` is clean

Current state of `register_tui_elements()` in `nullslop-component/src/lib.rs`:

```rust
pub fn register_tui_elements(registry: &mut AppUiRegistry) {
    registry.register(Box::new(chat_input_box::ChatInputBoxElement));
    registry.register(Box::new(chat_log::ChatLogElement));
    registry.register(Box::new(pinned_panel::PinnedPanelElement));
    registry.register(Box::new(
        provider::indicator::StreamingIndicatorElement::new(),
    ));
    registry.register(Box::new(provider::queue_element::QueueDisplayElement));
}
```

The 3 already-extracted elements (`StatusBarElement`, `CharCounterElement`, `DashboardElement`)
are **not** registered here — they're registered from their slice crates in `src/app.rs`. ✓

After Part B (chat-log extraction), `ChatLogElement` will also be removed from here.

### Part B: Extract `nsslice-chat-log`

#### 5. Create `crates/slices/nsslice-chat-log/Cargo.toml`

```toml
[package]
name = "nsslice-chat-log"
version = "0.1.0"
edition = "2024"

[dependencies]
nullslop-component-ui = { workspace = true }
nullslop-component = { workspace = true }
nullslop-protocol = { workspace = true }
ratatui = { workspace = true }

[dev-dependencies]
rstest = { workspace = true }

[lints]
workspace = true
```

Note: `nullslop-protocol` is needed because the element imports `ChatEntryKind`, `ChatEntry`,
and `PinPosition` from it. Verified by reading the element file imports.

#### 6. Create `crates/slices/nsslice-chat-log/src/lib.rs`

```rust
//! Chat log slice — renders the full conversation history.
//!
//! A display-only component showing all messages exchanged in the active session.
//! Each entry type has a distinct visual style (user bold with `>`, system dark gray,
//! actor yellow, assistant cyan). Supports scrolling, selection highlighting,
//! and pinned entry indicators.

pub mod element;

pub use element::ChatLogElement;

use nullslop_component::AppUiRegistry;

/// Register chat log UI element.
pub fn register(registry: &mut AppUiRegistry) {
    registry.register(Box::new(ChatLogElement));
}
```

#### 7. Create `crates/slices/nsslice-chat-log/src/element.rs`

Copy from `nullslop-component/src/chat_log/element.rs` with these import changes:

```rust
// Before (in nullslop-component):
use crate::AppState;

// After (in nsslice-chat-log):
use nullslop_component::AppState;
```

All other imports (`nullslop_component_ui::UiElement`, `nullslop_protocol::*`, `ratatui::*`)
remain the same — they're already external crate imports.

The tests module imports need updating:

```rust
// Before:
use crate::AppState;

// After:
use nullslop_component::AppState;
```

Everything else in the file stays the same. The file is 833 lines including tests (20 test functions).

#### 8. Delete old chat-log from `nullslop-component`

Delete `crates/nullslop-component/src/chat_log/element.rs`.

Update `crates/nullslop-component/src/chat_log/mod.rs`:

```rust
// Before:
pub mod element;
pub use element::ChatLogElement;

// After:
// (empty — delete the entire file and directory)
```

Since the module has nothing left after removing the element, delete the entire `chat_log`
directory and remove the module declaration from `lib.rs`.

**Action**:
- `rm -rf crates/nullslop-component/src/chat_log/`
- Remove `pub mod chat_log;` from `crates/nullslop-component/src/lib.rs`

#### 9. Update `nullslop-component/src/lib.rs`

Remove these lines:

```rust
// Remove from module declarations:
pub mod chat_log;

// Remove from register_tui_elements():
registry.register(Box::new(chat_log::ChatLogElement));
```

#### 10. Add workspace entries in root `Cargo.toml`

In `[workspace] members` list, add `"crates/slices/nsslice-chat-log"`:
```toml
members = ["crates/*", "crates/slices/nsslice-status-bar", "crates/slices/nsslice-char-counter", "crates/slices/nsslice-dashboard", "crates/slices/nsslice-chat-log", "actors/*", "tests/*"]
```

In `[workspace.dependencies]`, add:
```toml
nsslice-chat-log = { path = "crates/slices/nsslice-chat-log" }
```

#### 11. Add dependency in root `Cargo.toml` `[dependencies]`

Add `nsslice-chat-log = { workspace = true }` to the root package dependencies
(alongside the existing `nsslice-status-bar`, `nsslice-char-counter`, `nsslice-dashboard` entries).

#### 12. Wire registration in `src/app.rs`

Add `nsslice_chat_log::register(&mut ui_registry)` in both registration locations:

**TUI path** (~line 139, after `register_tui_elements`):
```rust
nullslop_component::register_tui_elements(&mut ui_registry);
nsslice_status_bar::register(&mut ui_registry);
nsslice_char_counter::register(&mut ui_registry);
nsslice_dashboard::register(&mut ui_registry);
nsslice_chat_log::register(&mut ui_registry);  // ADD
```

**Headless path** (~line 507, after `register_all`):
```rust
nullslop_component::register_all(&mut registry);
nsslice_status_bar::register(&mut registry);
nsslice_char_counter::register(&mut registry);
nsslice_dashboard::register(&mut registry);
nsslice_chat_log::register(&mut registry);  // ADD
```

#### 13. Verify

Run `cargo test` across the workspace. All existing tests should pass, and the new
`nsslice-chat-log` tests should run via `cargo test -p nsslice-chat-log`.

## Acceptance Criteria

1. **No duplicated code**: The directories `nullslop-component/src/{char_counter,status_bar}/`
   no longer exist. The file `nullslop-component/src/dashboard/element.rs` no longer exists.
   The directory `nullslop-component/src/chat_log/` no longer exists.
2. **No orphaned registrations**: `register_tui_elements()` in `nullslop-component/src/lib.rs`
   does not register `ChatLogElement` (or any of the 4 removed elements).
3. **All 4 slices registered**: `src/app.rs` registers `nsslice_status_bar`, `nsslice_char_counter`,
   `nsslice_dashboard`, and `nsslice_chat_log` in both the TUI and headless paths.
4. **`nsslice-chat-log` is independently testable**: `cargo test -p nsslice-chat-log` passes
   with all 20 chat-log element tests.
5. **Full workspace tests pass**: `cargo test` passes with no failures.
6. **`nullslop-tui` render dispatch unchanged**: `render.rs` still looks up `"chat-log"` by
   string name — no changes needed there.
7. **No circular dependencies**: `nsslice-chat-log` depends on `nullslop-component`, not the
   other way around.

## Files Changed

### Deleted
- `crates/nullslop-component/src/char_counter/` (entire directory)
- `crates/nullslop-component/src/status_bar/` (entire directory)
- `crates/nullslop-component/src/dashboard/element.rs`
- `crates/nullslop-component/src/chat_log/` (entire directory)

### Created
- `crates/slices/nsslice-chat-log/Cargo.toml`
- `crates/slices/nsslice-chat-log/src/lib.rs`
- `crates/slices/nsslice-chat-log/src/element.rs`

### Modified
- `Cargo.toml` (root) — workspace members + dependencies
- `crates/nullslop-component/src/lib.rs` — remove `chat_log` module + registration
- `src/app.rs` — add `nsslice_chat_log::register()` in 2 places
