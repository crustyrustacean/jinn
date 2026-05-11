# Phase 2: Create `nsslice-status-bar` crate (rendering-only)

## Context

This is the first real slice. The status bar is a pure rendering component — it has no intents, no validators, and no handlers. It reads `AppState` and draws text. This makes it the ideal candidate for the first migration: the pattern is simple (move code + wire up) and the risk is low.

The status bar currently lives in `nullslop-component/src/status_bar/` as two files: `mod.rs` (re-exports) and `element.rs` (the `StatusBarElement` struct, its `UiElement` impl, and all its tests).

## Files Involved

**Source (read):**
- `crates/nullslop-component/src/status_bar/element.rs` — the element impl and tests
- `crates/nullslop-component/src/status_bar/mod.rs` — re-exports
- `crates/nullslop-component/src/lib.rs` — where `StatusBarElement` is registered
- `crates/nullslop-component/Cargo.toml` — current dependencies
- `src/app.rs` — where the binary crate calls `register_tui_elements`
- `src/lib.rs` — binary crate module list (no changes needed)

**Target (create/modify):**
- `crates/slices/nsslice-status-bar/Cargo.toml` — new crate manifest
- `crates/slices/nsslice-status-bar/src/lib.rs` — new crate root with `register()`
- `crates/slices/nsslice-status-bar/src/element.rs` — moved element + tests
- `Cargo.toml` — add workspace dependency
- `crates/nullslop-component/src/lib.rs` — remove `status_bar` module + registration
- `src/app.rs` — add dependency + update registration calls
- `src/lib.rs` — add `wiring` module (optional, see below)

## Implementation Steps

### 1. Create the slice crate

Create `crates/slices/nsslice-status-bar/Cargo.toml`:

```toml
[package]
name = "nsslice-status-bar"
version = "0.1.0"
edition = "2024"

[dependencies]
nullslop-component-ui = { workspace = true }
nullslop-component = { workspace = true }
nullslop-providers = { workspace = true }
ratatui = { workspace = true }

[dev-dependencies]
rstest = { workspace = true }
nullslop-protocol = { workspace = true }

[lints]
workspace = true
```

The `element.rs` tests use `AppState`, `ProviderState`, and `PromptStrategyId` — these come from `nullslop-component` and `nullslop-protocol`. The `NO_PROVIDER_ID` constant comes from `nullslop-providers`.

### 2. Add to workspace dependencies

In root `Cargo.toml`, add to `[workspace.dependencies]`:

```toml
nsslice-status-bar = { path = "crates/slices/nsslice-status-bar" }
```

### 3. Create `src/element.rs`

Move the contents of `nullslop-component/src/status_bar/element.rs` to `crates/slices/nsslice-status-bar/src/element.rs`.

Update imports at the top of `element.rs`:
- `use crate::AppState;` → `use nullslop_component::AppState;`
- `use nullslop_component_ui::UiElement;` — unchanged (already absolute)
- `use nullslop_providers::NO_PROVIDER_ID;` — unchanged (already absolute)
- All `ratatui` imports — unchanged

In the test module:
- `use crate::{AppState, ProviderState};` → `use nullslop_component::{AppState, ProviderState};`
- `use nullslop_protocol::PromptStrategyId` — already used in test, just needs `nullslop-protocol` as dev-dep

### 4. Create `src/lib.rs`

```rust
//! Status bar slice — displays the active prompt strategy and current model.
//!
//! A display-only component at the bottom of the screen showing which
//! provider/model is active for the current session.

pub mod element;

pub use element::StatusBarElement;

use nullslop_component::AppUiRegistry;

/// Register the status bar UI element.
pub fn register(registry: &mut AppUiRegistry) {
    registry.register(Box::new(StatusBarElement));
}
```

### 5. Add `nsslice-status-bar` dependency to the binary crate

In root `Cargo.toml` (the `[dependencies]` section of the `nullslop` package), add:

```toml
nsslice-status-bar = { workspace = true }
```

### 6. Remove `status_bar` from `nullslop-component`

In `crates/nullslop-component/src/lib.rs`:
- Remove `pub mod status_bar;`
- Remove `registry.register(Box::new(status_bar::StatusBarElement));` from `register_tui_elements()`
- The `pub use` section does NOT re-export `StatusBarElement`, so no change needed there

### 7. Wire registration in the binary crate

There are **two** call sites in `src/app.rs` that register UI elements:

1. **Line ~139** (in `run_tui`): `nullslop_component::register_tui_elements(&mut ui_registry);`
2. **Line ~504** (in `build_core`): `nullslop_component::register_all(&mut registry);`

**Approach:** Add `nsslice_status_bar::register()` inline after each existing call:

```rust
nullslop_component::register_tui_elements(&mut ui_registry);
nsslice_status_bar::register(&mut ui_registry);
```

and:

```rust
nullslop_component::register_all(&mut registry);
nsslice_status_bar::register(&mut registry);
```

No wiring module is created yet — with only one slice, inline calls are clearer. The wiring module will be extracted after Phase 3 when there are 2+ slices.

### 8. Verify

```bash
just check
just test
```

## Acceptance Criteria

- [ ] `crates/slices/nsslice-status-bar/` exists with `Cargo.toml`, `src/lib.rs`, `src/element.rs`
- [ ] `nsslice-status-bar` is in workspace dependencies
- [ ] `nullslop-component` no longer has `pub mod status_bar;`
- [ ] `nullslop-component::register_tui_elements()` no longer registers `StatusBarElement`
- [ ] Binary crate depends on `nsslice-status-bar` and calls `nsslice_status_bar::register()`
- [ ] All status bar tests (9 tests) pass from the new crate
- [ ] `just check` succeeds with zero errors
- [ ] `just test` succeeds — all 1279+ tests pass

## Review Notes

**No security or performance implications.** This is a code move — the same bytes execute in the same order, just from a different crate.

**Dependency direction is correct:** `nsslice-status-bar` → `nullslop-component` + `nullslop-component-ui` + `nullslop-providers`. No cycles.

**The `nullslop-tui` crate** has its own test code calling `nullslop_component::register_all()` — after this phase, `register_all()` no longer registers `StatusBarElement`. This is safe because those tests don't assert on status bar rendering; they just need *some* elements in the registry.

**Potential issue:** The element's tests construct `ProviderState` directly (e.g., `ProviderState { active_provider: "ollama/llama3".to_owned(), ..ProviderState::default() }`). This works because `ProviderState`'s fields are `pub`. No change needed.

**`nullslop-tui` tests:** `nullslop-tui/src/app.rs` calls `nullslop_component::register_all()` in 3 test helpers (lines 405, 601, 919). After removing `StatusBarElement` from `register_tui_elements()`, these tests won't have the status bar registered. This is safe — those tests don't assert on status bar rendering; they just need *some* elements in the registry. No action needed.

**Wiring module deferred:** The original high-level plan called for creating `src/wiring.rs`. This is deferred because: (a) with only one slice, adding `nsslice_status_bar::register()` inline at both `src/app.rs` call sites is clearer and less indirection; (b) `nullslop-tui` tests can't use a binary-crate wiring module — they call `nullslop_component::register_all()` directly; (c) the wiring function will be more naturally extracted after Phase 3 when there are 2+ slices and the pattern is established.
