# Phase 9: Migrate `PinnedPanelState`

## Problem

`PinnedPanelState` (297 lines) lives in `nullslop-component/src/pinned_panel/state.rs`. It's owned by the pinned panel slice. Moving it into `nsslice-pinned-panel-protocol` shrinks `nullslop-component`.

## What Moves

- `PinnedPanelState`, `PinnedEntry`, all tests → `nsslice-pinned-panel-protocol/src/lib.rs`

## What Stays

- `nsslice-pinned-panel/src/` — stays, updates imports
- `AppState` field `pinned_panel: PinnedPanelState` — stays, typed via protocol crate re-export

## File Changes

### 1. NEW `crates/slices/nsslice-pinned-panel-protocol/Cargo.toml`
```toml
[package]
name = "nsslice-pinned-panel-protocol"
version = "0.1.0"
edition = "2024"

[dependencies]
nullslop-protocol = { workspace = true }

[dev-dependencies]
rstest = { workspace = true }

[lints]
workspace = true
```

### 2. NEW `crates/slices/nsslice-pinned-panel-protocol/src/lib.rs`
Copy from `nullslop-component/src/pinned_panel/state.rs`. Update `use crate::ChatEntryId` → `use nullslop_protocol::ChatEntryId`.

### 3. MODIFY `crates/nullslop-component/Cargo.toml`
Add: `nsslice-pinned-panel-protocol = { workspace = true }`

### 4. MODIFY `crates/nullslop-component/src/lib.rs`
Remove `pub mod pinned_panel;`, change re-export to `pub use nsslice_pinned_panel_protocol::PinnedPanelState;`

### 5. MODIFY `crates/nullslop-component/src/app_state.rs`
Change `use crate::pinned_panel::PinnedPanelState;` to `pub use nsslice_pinned_panel_protocol::PinnedPanelState;`

### 6. MODIFY `crates/slices/nsslice-pinned-panel/Cargo.toml`
Add: `nsslice-pinned-panel-protocol = { workspace = true }`

### 7. MODIFY `crates/slices/nsslice-pinned-panel/src/` files — update imports

### 8. MODIFY root `Cargo.toml` — add to members and workspace deps

### 9. DELETE `crates/nullslop-component/src/pinned_panel/` directory

- [x] `crates/slices/nsslice-pinned-panel-protocol/` exists with Cargo.toml and src/lib.rs
- [x] `crates/nullslop-component/src/pinned_panel/` is deleted
- [x] `nullslop-component` re-exports `PinnedPanelState` from protocol crate
- [x] `nsslice-pinned-panel` imports types from protocol crate
- [x] `just check` passes
- [x] `just test` passes

---

## Review: Phase 9 — Migrate `PinnedPanelState`

### Changes

- Created `nsslice-pinned-panel-protocol` with `PinnedPanelState`, `PinnedEntry`, and all tests (297 lines)
- Updated `nullslop-component` to import and re-export from protocol crate
- Deleted `nullslop-component/src/pinned_panel/` directory

### Divergence Summary

- None.

### Verification

- `just check` — zero errors
- `just test` — all pass

### Risks

- None.

### Next Steps

Proceed to Phase 10: Migrate `ChatInputBoxState`.
