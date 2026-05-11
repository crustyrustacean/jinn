# Phase 8: Migrate `DashboardState`

## Problem

`DashboardState` (309 lines) lives in `nullslop-component/src/dashboard/state.rs`. It's owned by the dashboard slice. Moving it into `nsslice-dashboard-protocol` shrinks `nullslop-component` and demonstrates the state migration pattern for display-only slices.

## What Moves

- `DashboardState`, `ActorStatus`, `TrackedActor`, all 9 tests → `nsslice-dashboard-protocol/src/lib.rs`

## What Stays

- `nsslice-dashboard/src/element.rs` — stays, just updates its import
- `AppState` field `dashboard: DashboardState` — stays, now typed via protocol crate re-export

## File Changes

### 1. NEW `crates/slices/nsslice-dashboard-protocol/Cargo.toml`

```toml
[package]
name = "nsslice-dashboard-protocol"
version = "0.1.0"
edition = "2024"

[lints]
workspace = true

[dev-dependencies]
rstest = { workspace = true }
```

### 2. NEW `crates/slices/nsslice-dashboard-protocol/src/lib.rs`

Copy from `nullslop-component/src/dashboard/state.rs` (309 lines + tests). No import changes needed — only uses `std::collections::HashMap`.

### 3. MODIFY `crates/nullslop-component/Cargo.toml`

Add dependency:
```toml
nsslice-dashboard-protocol = { workspace = true }
```

### 4. MODIFY `crates/nullslop-component/src/lib.rs`

Remove `pub mod dashboard;` and change re-export:
```rust
// Before:
pub use dashboard::DashboardState;
// After:
pub use nsslice_dashboard_protocol::DashboardState;
```

### 5. MODIFY `crates/nullslop-component/src/app_state.rs`

Change import:
```rust
// Before:
use crate::dashboard::DashboardState;
// After:
pub use nsslice_dashboard_protocol::DashboardState;
```

### 6. MODIFY `crates/slices/nsslice-dashboard/Cargo.toml`

Add dependency:
```toml
nsslice-dashboard-protocol = { workspace = true }
```

### 7. MODIFY `crates/slices/nsslice-dashboard/src/element.rs`

Change import:
```rust
// Before:
use nullslop_component::dashboard::state::ActorStatus;
// After:
use nsslice_dashboard_protocol::ActorStatus;
```

### 8. MODIFY root `Cargo.toml`

Add to workspace members, `[workspace.dependencies]`.

### 9. DELETE `crates/nullslop-component/src/dashboard/` directory

## Acceptance Criteria

- [ ] `crates/slices/nsslice-dashboard-protocol/` exists with `Cargo.toml` and `src/lib.rs`
- [ ] `crates/nullslop-component/src/dashboard/` is deleted
- [ ] `nullslop-component` re-exports `DashboardState` from protocol crate
- [ ] `nsslice-dashboard` imports `ActorStatus` from protocol crate
- [ ] `just check` passes
- [ ] `just test` passes — all 9 dashboard state tests pass from protocol crate

---

## Review: Phase 8 — Migrate `DashboardState`

### Changes

- Created `nsslice-dashboard-protocol` with `DashboardState`, `ActorStatus`, `TrackedActor`, and 9 tests (309 lines)
- Updated `nullslop-component` to import and re-export from protocol crate
- Updated `nsslice-dashboard` element to import `ActorStatus` from protocol crate
- Deleted `nullslop-component/src/dashboard/` directory

### Divergence Summary

- Protocol crate needed `rstest` as a dev-dependency (tests use `#[rstest::rstest]`)

### Verification

- `just check` — zero errors
- `just test` — all pass (including dashboard state tests in protocol crate)

### Risks

- None. `DashboardState` has zero external dependencies (only `std`).

### Next Steps

Proceed to Phase 9: Migrate `PinnedPanelState`.
