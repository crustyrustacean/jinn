# Phase 1: Extract Dashboard Intents → `nsslice-dashboard`

## Problem

`nsslice-dashboard` already has its element extracted (from VSA phase 1), but the 4 dashboard intent handlers still live in `nullslop-intent/src/handler.rs`. The dashboard validator file (`nullslop-intent/src/validators/dashboard.rs`) is empty — just a doc comment. This phase completes the dashboard slice by moving intent handling into it and cleaning up the empty validator.

## What Moves

### Handler match arms → `nsslice-dashboard/src/intent.rs`

From `nullslop-intent/src/handler.rs` (lines 296–312), these 4 match arms:

```rust
Intent::DashboardSelectDown => {
    state.frontend.dashboard.select_next();
    IntentResult::empty()
}
Intent::DashboardSelectUp => {
    state.frontend.dashboard.select_prev();
    IntentResult::empty()
}
Intent::DashboardSelectFirst => {
    state.frontend.dashboard.select_first();
    IntentResult::empty()
}
Intent::DashboardSelectLast => {
    state.frontend.dashboard.select_last();
    IntentResult::empty()
}
```

Become 4 public handler functions in the slice:

```rust
pub fn handle_select_down(state: &mut AppState) -> IntentResult {
    state.frontend.dashboard.select_next();
    IntentResult::empty()
}

pub fn handle_select_up(state: &mut AppState) -> IntentResult {
    state.frontend.dashboard.select_prev();
    IntentResult::empty()
}

pub fn handle_select_first(state: &mut AppState) -> IntentResult {
    state.frontend.dashboard.select_first();
    IntentResult::empty()
}

pub fn handle_select_last(state: &mut AppState) -> IntentResult {
    state.frontend.dashboard.select_last();
    IntentResult::empty()
}
```

### Tests → `nsslice-dashboard/src/intent.rs`

4 tests move from `nullslop-intent/src/handler_tests.rs` (lines 909–975):
- `dashboard_select_down_moves_selection`
- `dashboard_select_up_moves_selection`
- `dashboard_select_first_moves_to_first`
- `dashboard_select_last_moves_to_last`

Tests will call the slice functions directly instead of going through `IntentHandler::handle()`. Import changes:
- Remove: `use super::IntentHandler;`, `use crate::Intent;`
- Remove: `use nullslop_protocol::ChatEntry;` (unused by dashboard tests)
- Add: `use nullslop_component::AppState;`
- Add: `use nullslop_protocol::IntentResult;`

### Validator: delete (empty file)

`nullslop-intent/src/validators/dashboard.rs` contains only a module-level doc comment — no functions, no types, no tests. Delete the file and remove `pub mod dashboard;` from `nullslop-intent/src/validators/mod.rs`.

## What Stays

- The dashboard element (`element.rs`) — already in the slice, unchanged.
- The `register()` function — already in `lib.rs`, unchanged.
- `DashboardState` — stays in `nullslop-component` (state struct, no circular dep).

## File Changes

### 1. `crates/slices/nsslice-dashboard/Cargo.toml`

Add `nullslop-protocol` dependency (needed for `IntentResult`):

```toml
[dependencies]
nullslop-component-ui = { workspace = true }
nullslop-component = { workspace = true }
nullslop-protocol = { workspace = true }
ratatui = { workspace = true }
```

No new dev-dependencies needed (`rstest` already present).

### 2. `crates/slices/nsslice-dashboard/src/lib.rs`

Add `pub mod intent;`:

```rust
pub mod element;
pub mod intent;

pub use element::DashboardElement;

use nullslop_component::AppUiRegistry;

pub fn register(registry: &mut AppUiRegistry) {
    registry.register(Box::new(DashboardElement));
}
```

### 3. NEW `crates/slices/nsslice-dashboard/src/intent.rs`

Create with the 4 handler functions + 4 moved tests. Module doc comment should explain the 4 intents handled here.

### 4. `crates/nullslop-intent/src/handler.rs`

Replace the 4 inline match arms with delegations:

```rust
// --- Dashboard ---
Intent::DashboardSelectDown => {
    nsslice_dashboard::intent::handle_select_down(state)
}
Intent::DashboardSelectUp => {
    nsslice_dashboard::intent::handle_select_up(state)
}
Intent::DashboardSelectFirst => {
    nsslice_dashboard::intent::handle_select_first(state)
}
Intent::DashboardSelectLast => {
    nsslice_dashboard::intent::handle_select_last(state)
}
```

No import changes needed — `nsslice_dashboard` is already a dependency (from VSA phase 1 element extraction, the `nullslop-tui` dep graph already includes it). However, `nullslop-intent/Cargo.toml` may need `nsslice-dashboard` added as a dependency. Check:

### 5. `crates/nullslop-intent/Cargo.toml`

Add `nsslice-dashboard` dependency (needed for `nsslice_dashboard::intent::*` calls):

```toml
nsslice-dashboard = { workspace = true }
```

### 6. DELETE `crates/nullslop-intent/src/validators/dashboard.rs`

Empty file — just delete.

### 7. `crates/nullslop-intent/src/validators/mod.rs`

Remove `pub mod dashboard;` line:

```rust
pub mod app;
pub mod chat_entry;
pub mod picker;
```

### 8. `crates/nullslop-intent/src/handler_tests.rs`

Remove the 4 dashboard tests (lines 909–975): the `// ============ Dashboard Intents ============` section comment and the 4 `#[rstest::rstest] fn dashboard_select_*` functions.

## Acceptance Criteria

1. `nsslice-dashboard/src/intent.rs` exists with 4 public handler functions
2. 4 dashboard tests pass in the slice (`cargo test -p nsslice-dashboard`)
3. `nullslop-intent/src/validators/dashboard.rs` is deleted
4. `nullslop-intent/src/validators/mod.rs` no longer references `dashboard`
5. `nullslop-intent/handler.rs` delegates 4 dashboard match arms to slice
6. `cargo test --workspace` passes — no regressions

## Risks

- **Trivial phase** — all 4 intents are 2-line functions with no validators, no conditional logic, no commands returned. Very low risk.
- No security or performance implications — purely organizational change.
