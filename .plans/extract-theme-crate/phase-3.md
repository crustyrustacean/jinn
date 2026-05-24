# Phase 3: Wire up `nullslop-domain` → `nullslop-theme`

## Problem
Connect the new `nullslop-theme` crate to `nullslop-domain` so all existing consumers continue to work.

## Changes
- Added `nullslop-theme` dependency to `nullslop-domain/Cargo.toml`
- Replaced `feat/theme/mod.rs` with a re-export facade
- Deleted all original theme source files from `nullslop-domain/src/feat/theme/` (kept only `mod.rs`)
- Re-exported `contrast` module for direct path access (`crate::feat::theme::contrast::darken`)
- Fixed orphan rule violation: `impl RichTextTheme for Theme` → newtype `MarkdownTheme<'a>` wrapper
- Inlined `Generation(1)` in `history.rs` where `theme.generation()` was called

## Acceptance Criteria
- [x] `cargo check -p nullslop-domain` passes
- [x] `cargo test -p nullslop-domain` — all tests pass
- [x] `feat/theme/mod.rs` is purely re-exports
- [x] Original theme source files deleted from domain

---

## Review: Phase 3 — Wire up `nullslop-domain` → `nullslop-theme`

### Changes
Wired up the re-export facade and handled the orphan rule for `RichTextTheme`.

### Divergence Summary
- **Orphan rule fix:** The `impl RichTextTheme for Theme` in `markdown.rs` violated Rust's orphan rule since both `Theme` (now in `nullslop-theme`) and `RichTextTheme` (from `ratatui-markdown`) are external types. Fixed by introducing a newtype `MarkdownTheme<'a>(&'a Theme)` wrapper.
- **`generation()` method:** Was previously available on `Theme` via the `RichTextTheme` impl. Replaced with inline `Generation(1)` at the single call site in `history.rs`.
- **`contrast` module re-export:** Added `pub use nullslop_theme::contrast;` to the facade for direct submodule access (`crate::feat::theme::contrast::darken`).

### Verification
- `cargo check -p nullslop-domain` — compiles clean
- `cargo test -p nullslop-domain` — all tests pass

### Risks
The `MarkdownTheme` wrapper is a local newtype that delegates to `Theme`. If `RichTextTheme` adds new trait methods, the wrapper will need updating.

### Next Steps
Proceeding to Phase 4: verify external consumers compile.
