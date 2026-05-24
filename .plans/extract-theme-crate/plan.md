# extract-theme-crate — High-Level Plan

## Problem

The theme system (types, loading, contrast, color parsing, picker entry) lives inside `nullslop-domain`, which is a large crate (~200 source files) with many heavy dependencies (diesel, tiktoken, tokio, tree-sitter, etc.). The theme code has a narrow dependency footprint (`ratatui`, `serde`, `toml`, `anstyle`, `wherror`, `error-stack`, `nullslop-selection-widget`) and is conceptually independent. Extracting it into its own crate improves compile times, dependency clarity, and reusability.

## Solution

Create a new `nullslop-theme` crate under `crates/nullslop-theme/` containing the 7 source files currently in `nullslop-domain/src/feat/theme/`. Move `ThemeEntry` into the new crate alongside all other theme types. Replace the theme module in `nullslop-domain` with a thin re-export facade so all ~35 internal consumers continue to work via `crate::feat::theme::*`. External consumers (`nullslop-tui`, root `src/app.rs`) use the re-export path and require no changes. The `themes/` directory stays at the workspace root.

---

## Phases

- [x] **Phase 1: Scaffold the new crate**
  - Created `crates/nullslop-theme/Cargo.toml`
  - Added `nullslop-theme` to workspace dependencies in root `Cargo.toml`
  - Created `crates/nullslop-theme/src/lib.rs`
  - Verified: `cargo check -p nullslop-theme`

- [x] **Phase 2: Move theme files**
  - Moved all 7 source files + 1 test file to `nullslop-theme/src/`
  - Fixed `include_str!` path in `default_theme.rs`
  - Fixed all `super::` → `crate::` imports
  - Verified: `cargo check -p nullslop-theme` and `cargo test -p nullslop-theme` (65 tests)

- [x] **Phase 3: Wire up `nullslop-domain` → `nullslop-theme`**
  - Added `nullslop-theme` dependency to `nullslop-domain/Cargo.toml`
  - Replaced `feat/theme/mod.rs` with re-export facade
  - Deleted original theme source files from `nullslop-domain/src/feat/theme/`
  - Fixed orphan rule violation with `MarkdownTheme` newtype wrapper
  - Verified: `cargo check -p nullslop-domain`, `cargo test -p nullslop-domain`

- [x] **Phase 4: Verify external consumers**
  - Verified `nullslop-tui` compiles
  - Verified `src/app.rs` compiles
  - Verified: `just check` on full workspace

- [x] **Phase 5: Verify**
  - `just check` — full workspace ✓
  - `cargo test -p nullslop-theme` — 65 tests pass ✓
  - `just test` — full suite passes ✓
  - `just clippy` — no new warnings ✓

---

## Acceptance Criteria

- [x] `just check` passes for the entire workspace
- [x] All existing tests in `nullslop-theme` pass (65 tests)
- [x] All existing tests in `nullslop-domain` pass (no regressions)
- [x] `nullslop-domain` no longer directly contains theme logic — its `feat/theme/mod.rs` is purely re-exports from `nullslop-theme`
- [x] The `themes/` directory remains at the workspace root
- [x] `ThemeEntry` (with `PickerItem` impl) lives in `nullslop-theme`
