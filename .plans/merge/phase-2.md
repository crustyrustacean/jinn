# Phase 2: Create `nsslice-shutdown`

Move shutdown tracker actor + create first `-protocol` crate for `ShutdownTrackerState`. Proves the protocol crate pattern.

## Steps

1. Create `crates/slices/nsslice-shutdown-protocol/` with `Cargo.toml` (minimal deps) and `src/lib.rs` containing `ShutdownTrackerState`
2. Update `nullslop-component` to depend on `nsslice-shutdown-protocol`, import `ShutdownTrackerState` from there
3. Delete `nullslop-component/src/shutdown_tracker/` directory and `pub mod shutdown_tracker;` from lib.rs
4. Create `crates/slices/nsslice-shutdown/` with Cargo.toml, move actor from `actors/nullslop-shutdown-tracker`
5. Add `nsslice-shutdown` and `nsslice-shutdown-protocol` to root `Cargo.toml`
6. Update `src/app.rs` imports
7. Delete `actors/nullslop-shutdown-tracker/`
8. Run `just check` then `just test`

## Acceptance Criteria

- [x] `nsslice-shutdown-protocol` crate exists with `ShutdownTrackerState`
- [x] `nsslice-shutdown` crate exists with the shutdown tracker actor (including its tests)
- [x] `nullslop-component/src/shutdown_tracker/` is deleted
- [x] `nullslop-component` imports `ShutdownTrackerState` from `nsslice-shutdown-protocol`
- [x] `actors/nullslop-shutdown-tracker/` is deleted
- [x] `just check` passes
- [x] `just test` passes

---

## Review: Phase 2 — Create `nsslice-shutdown`

### Changes

- Created `crates/slices/nsslice-shutdown-protocol/` with `ShutdownTrackerState` (moved from `nullslop-component/src/shutdown_tracker/`)
- Created `crates/slices/nsslice-shutdown/` with the shutdown tracker actor (moved from `actors/nullslop-shutdown-tracker/`)
- Updated `nullslop-component` to depend on `nsslice-shutdown-protocol` and import `ShutdownTrackerState` from there
- Deleted `nullslop-component/src/shutdown_tracker/` directory
- Deleted `actors/nullslop-shutdown-tracker/` directory
- Updated root `Cargo.toml` and `src/app.rs` imports

### Divergence Summary

- Had to be careful with Cargo.toml duplicate keys when replacing dependencies — the `[dependencies]` section already had `nullslop-providers` so replacing `nullslop-shutdown-tracker` with `nullslop-providers` caused a duplicate.

### Verification

- `just check` — zero errors
- `just test` — all pass (including e2e cucumber tests)

### Risks

- None. The protocol crate pattern is proven with zero dependencies — `ShutdownTrackerState` only uses `std`.

### Next Steps

Proceed to Phase 3: Create `nsslice-llm`.
