# Phase 1–2: Scaffold and move theme files

## Problem
Need to create the `nullslop-theme` crate and move all theme files from `nullslop-domain`.

## Changes
- Created `crates/nullslop-theme/Cargo.toml` with all needed dependencies
- Added `nullslop-theme` to workspace dependencies in root `Cargo.toml`
- Created `crates/nullslop-theme/src/lib.rs` re-exporting the public API
- Moved 7 source files + 1 test file from `nullslop-domain/src/feat/theme/`
- Fixed `include_str!` path in `default_theme.rs`
- Fixed all `super::` → `crate::` imports in moved files
- Fixed `crate::feat::theme::Theme` → `crate::Theme` in `theme_entry.rs`

## Acceptance Criteria
- [x] `cargo check -p nullslop-theme` passes
- [x] All 65 theme tests pass (`cargo test -p nullslop-theme`)

---

## Review: Phase 1–2 — Scaffold and move theme files

### Changes
Scaffolded the new `nullslop-theme` crate and moved all theme source files with import path fixes.

### Divergence Summary
Used explicit `[lints.clippy]` overrides instead of `[lints] workspace = true` to match the pattern used by `nullslop-domain` (which avoids inheriting `missing_docs = "deny"` for struct fields that lack doc comments).

### Verification
- `cargo check -p nullslop-theme` — compiles
- `cargo test -p nullslop-theme` — 65 tests pass

### Risks
None. Files are exact copies with only import path changes.

### Next Steps
Proceeding to Phase 3: wire up `nullslop-domain` → `nullslop-theme` re-export.
