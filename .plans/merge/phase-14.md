# Phase 14: Move non-slice crates into `crates/common/`

## Problem

Reorganize directory structure so the workspace has two clear top-level groups under `crates/`: `common/` and `slices/`.

## What Moves

All non-slice crates from `crates/` into `crates/common/`:
- `nullslop-protocol`, `nullslop-protocol-derive`, `nullslop-component`, `nullslop-component-ui`
- `nullslop-core`, `nullslop-intent`, `nullslop-services`, `nullslop-tui`
- `nullslop-actor`, `nullslop-actor-host`, `nullslop-cli`, `nullslop-providers`
- `nullslop-selection-widget`, `nullslop-workflow`, `nullslop-prompt-template`

## File Changes

### 1. Create `crates/common/` directory
### 2. Move each crate directory
### 3. Update all `path = "crates/..."` references in root Cargo.toml to `path = "crates/common/..."`
### 4. Update workspace members glob
### 5. Update `ARCHITECTURE.md` and `AGENTS.md` if needed
### 6. Run `just check` then `just test`

- [x] All common crates are in `crates/common/`
- [x] All slice crates remain in `crates/slices/`
- [x] Root `Cargo.toml` paths updated
- [x] `just check` passes
- [x] `just test` passes

---

## Review: Phase 14 — Move non-slice crates into `crates/common/`

### Changes

- Created `crates/common/` directory
- Moved all 15 non-slice crates from `crates/` into `crates/common/`
- Updated all path references in root Cargo.toml from `crates/nullslop-*` to `crates/common/nullslop-*`
- Simplified workspace members to `members = ["crates/common/*", "crates/slices/*", "tests/*"]`
- Removed the old `exclude = ["crates/slices"]` that was no longer needed

### Divergence Summary

- None.

### Verification

- `just check` — zero errors
- `just test` — all pass

### Risks

- None. The directory structure now clearly separates common crates from slice crates.

### Next Steps

All phases complete. Implementation done.
