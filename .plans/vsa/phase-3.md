# Phase 3: Extract `nsslice-provider` slice

This phase extracts the two provider UI elements (`StreamingIndicatorElement` and
`QueueDisplayElement`) from `nullslop-component/src/provider/` into a new slice crate
at `crates/slices/nsslice-provider/`.

This is a display-only extraction — no intents, no validators, no state changes. Both
elements read from `AppState` and render. The pattern is identical to the 4 slices
already extracted (char-counter, status-bar, dashboard, chat-log).

## Context

The `provider` module in `nullslop-component` currently has 3 files:
- `mod.rs` — declares `pub mod indicator; pub mod queue_element;`
- `indicator.rs` — `StreamingIndicatorElement` (animated throbber during streaming/sending)
- `queue_element.rs` — `QueueDisplayElement` (dimmed "QUEUED:" entries)

The module contains no state structs and no handler code. Both elements are pure rendering.
The entire module can be deleted from `nullslop-component` after extraction.

### Dependencies of the element files

**`indicator.rs`** uses:
- `nullslop_component_ui::UiElement`
- `nullslop_component::AppState` (currently `crate::AppState`)
- `ratatui::*`
- `throbber_widgets_tui::*`

**`queue_element.rs`** uses:
- `nullslop_component_ui::UiElement`
- `nullslop_component::AppState` (currently `crate::AppState`)
- `ratatui::*`
- `unicode_segmentation::UnicodeSegmentation`

So the slice crate needs dependencies: `nullslop-component-ui`, `nullslop-component`,
`ratatui`, `throbber-widgets-tui`, `unicode-segmentation`.

### Key difference from previous slices

`StreamingIndicatorElement` is **not a unit struct** — it has internal state
(`throbber_state: ThrobberState`) and a `new()` constructor. The `register()` function
must call `StreamingIndicatorElement::new()` (not `StreamingIndicatorElement`).

`QueueDisplayElement` is a unit struct — same as all previous slices.

## Detailed Steps

### 1. Create `crates/slices/nsslice-provider/Cargo.toml`

```toml
[package]
name = "nsslice-provider"
version = "0.1.0"
edition = "2024"

[dependencies]
nullslop-component-ui = { workspace = true }
nullslop-component = { workspace = true }
ratatui = { workspace = true }
throbber-widgets-tui = { workspace = true }
unicode-segmentation = { workspace = true }

[dev-dependencies]
rstest = { workspace = true }

[lints]
workspace = true
```

Note: `throbber-widgets-tui` and `unicode-segmentation` are new dependencies not needed
by the previous display-only slices. Both are already in the workspace dependencies.

### 2. Create `crates/slices/nsslice-provider/src/lib.rs`

```rust
//! Provider slice — streaming indicator and message queue display.
//!
//! Two display-only elements:
//!
//! - **Streaming indicator** — animated throbber shown during sending/streaming.
//! - **Queue display** — dimmed "QUEUED:" entries for messages waiting in the queue.

pub mod indicator;
pub mod queue_element;

pub use indicator::StreamingIndicatorElement;
pub use queue_element::QueueDisplayElement;

use nullslop_component::AppUiRegistry;

/// Register provider UI elements.
pub fn register(registry: &mut AppUiRegistry) {
    registry.register(Box::new(StreamingIndicatorElement::new()));
    registry.register(Box::new(QueueDisplayElement));
}
```

### 3. Create `crates/slices/nsslice-provider/src/indicator.rs`

Copy from `nullslop-component/src/provider/indicator.rs` with one import change:

```rust
// Before:
use crate::AppState;

// After:
use nullslop_component::AppState;
```

All other imports are already absolute (`nullslop_component_ui::UiElement`, `ratatui::*`,
`throbber_widgets_tui::*`) and remain unchanged.

The file has 1 test (`name_returns_streaming_indicator`).

### 4. Create `crates/slices/nsslice-provider/src/queue_element.rs`

Copy from `nullslop-component/src/provider/queue_element.rs` with one import change:

```rust
// Before:
use crate::AppState;

// After:
use nullslop_component::AppState;
```

All other imports remain unchanged. This file has no tests.

### 5. Delete `crates/nullslop-component/src/provider/` (entire directory)

```bash
rm -rf crates/nullslop-component/src/provider/
```

### 6. Update `crates/nullslop-component/src/lib.rs`

Remove:
- `pub mod provider;` from the module declarations
- The two registration lines from `register_tui_elements()`:
  ```rust
  registry.register(Box::new(
      provider::indicator::StreamingIndicatorElement::new(),
  ));
  registry.register(Box::new(provider::queue_element::QueueDisplayElement));
  ```

After this, `register_tui_elements()` will only register:
- `chat_input_box::ChatInputBoxElement`
- `pinned_panel::PinnedPanelElement`

### 7. Update root `Cargo.toml`

**`[workspace] members`** — add `"crates/slices/nsslice-provider"` to the list.

**`[workspace.dependencies]`** — add:
```toml
nsslice-provider = { path = "crates/slices/nsslice-provider" }
```

**`[dependencies]`** (root package) — add:
```toml
nsslice-provider = { workspace = true }
```

### 8. Wire registration in `src/app.rs`

Add `nsslice_provider::register(&mut ui_registry)` at both existing registration sites:

**TUI path** (~line 143, after the other slice registrations):
```rust
nullslop_component::register_tui_elements(&mut ui_registry);
nsslice_status_bar::register(&mut ui_registry);
nsslice_char_counter::register(&mut ui_registry);
nsslice_dashboard::register(&mut ui_registry);
nsslice_chat_log::register(&mut ui_registry);
nsslice_provider::register(&mut ui_registry);  // ADD
```

**Headless path** (~line 512, after the other slice registrations):
```rust
nullslop_component::register_all(&mut registry);
nsslice_status_bar::register(&mut registry);
nsslice_char_counter::register(&mut registry);
nsslice_dashboard::register(&mut registry);
nsslice_chat_log::register(&mut registry);
nsslice_provider::register(&mut registry);  // ADD
```

### 9. Update `crates/nullslop-tui/Cargo.toml`

Add `nsslice-provider = { workspace = true }` to dependencies (alongside the existing
slice dependencies).

### 10. Update `crates/nullslop-tui/src/app.rs`

Add `nsslice_provider::register()` at all 3 `nullslop_component::register_all()` call
sites (lines ~409, ~609, ~931). Insert after the last existing slice registration
(`nsslice_chat_log::register()`).

### 11. Verify

```bash
cargo test --workspace
cargo test -p nsslice-provider
```

## Acceptance Criteria

1. **No duplicated code**: `crates/nullslop-component/src/provider/` directory no longer exists
2. **No orphaned registrations**: `register_tui_elements()` in `nullslop-component/src/lib.rs` does not register `StreamingIndicatorElement` or `QueueDisplayElement`
3. **Slice registered everywhere**: `nsslice_provider::register()` is called in all 5 registration sites:
   - `src/app.rs` TUI path (1 site)
   - `src/app.rs` headless path (1 site)
   - `crates/nullslop-tui/src/app.rs` (3 sites)
4. **`nsslice-provider` is independently testable**: `cargo test -p nsslice-provider` passes (1 test: `name_returns_streaming_indicator`)
5. **Full workspace tests pass**: `cargo test --workspace` passes with no failures
6. **Element names unchanged**: Elements still return `"streaming-indicator"` and `"queue-display"` — `nullslop-tui/render.rs` still finds them by name
7. **No circular dependencies**: `nsslice-provider` depends on `nullslop-component`, not the other way around

## Files Changed

### Deleted
- `crates/nullslop-component/src/provider/` (entire directory: `mod.rs`, `indicator.rs`, `queue_element.rs`)

### Created
- `crates/slices/nsslice-provider/Cargo.toml`
- `crates/slices/nsslice-provider/src/lib.rs`
- `crates/slices/nsslice-provider/src/indicator.rs`
- `crates/slices/nsslice-provider/src/queue_element.rs`

### Modified
- `Cargo.toml` (root) — workspace members + dependencies
- `crates/nullslop-component/src/lib.rs` — remove `provider` module + 2 registration lines
- `crates/nullslop-tui/Cargo.toml` — add `nsslice-provider` dependency
- `crates/nullslop-tui/src/app.rs` — add `nsslice_provider::register()` at 3 sites
- `src/app.rs` — add `nsslice_provider::register()` at 2 sites
