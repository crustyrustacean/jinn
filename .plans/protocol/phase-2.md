# Phase 2: Move `nullslop-protocol` into `nullslop-domain/src/protocol/`

## Problem

The 59 files in `nullslop-protocol/src/` need to move into `nullslop-domain/src/protocol/`. All internal `crate::` references need updating, and the protocol's public types need re-exporting from `nullslop-domain`'s crate root.

## What Moves / What Stays

**Moves:** All 59 source files from `nullslop-protocol/src/` → `nullslop-domain/src/protocol/`

**Stays:** `nullslop-protocol` crate itself stays (consumed by downstream crates until Phase 9). `nullslop-protocol-derive` stays untouched.

## Implementation Strategy

Rather than copying files and fixing references, we'll:
1. Copy all files from `nullslop-protocol/src/` into `nullslop-domain/src/protocol/`
2. Replace the stub `protocol/mod.rs` with `nullslop-protocol`'s `lib.rs` content (adapted)
3. Fix all `crate::` references to `crate::protocol::`
4. Add re-exports from the domain crate root `lib.rs`
5. Verify `cargo check -p nullslop-domain` passes

## File Changes

### 1. Copy all files

```
nullslop-protocol/src/*.rs → nullslop-domain/src/protocol/*.rs
nullslop-protocol/src/**/*.rs → nullslop-domain/src/protocol/**/*.rs
```

### 2. Replace `protocol/mod.rs` with adapted `nullslop-protocol/src/lib.rs`

The `lib.rs` becomes `protocol/mod.rs`. All `pub mod` declarations stay. All re-exports stay. The module-level doc comment is preserved.

### 3. Fix `crate::` → `crate::protocol::`

In all moved files, any `crate::` reference needs to become `crate::protocol::`. This includes:
- `use crate::ChatEntry` → `use crate::protocol::ChatEntry`
- `use crate::provider::` → `use crate::protocol::provider::`
- `use crate::session::SessionId` → `use crate::protocol::session::SessionId`
- etc.

Files that use `super::` references should be fine since the relative module tree structure is preserved.

### 4. Update `nullslop-domain/src/lib.rs`

Add re-exports for all public types currently re-exported by `nullslop-protocol/src/lib.rs`.

### 5. Handle `which-key` feature

The `ratatui_which_key::Key` impl in `key.rs` is behind `#[cfg(feature = "which-key")]`. This should work as-is since `nullslop-domain` has the same feature flag.

## Implementation Order

1. Copy all files
2. Replace `protocol/mod.rs` with adapted `lib.rs`
3. Bulk-replace `crate::` references
4. Add re-exports to domain `lib.rs`
5. `cargo check -p nullslop-domain`
6. `cargo test -p nullslop-domain`

## Acceptance Criteria

- [x] All 59 files from `nullslop-protocol/src/` exist in `nullslop-domain/src/protocol/`
- [x] `protocol/mod.rs` has proper module declarations matching the original `lib.rs`
- [x] All `crate::` references in moved files updated to `crate::protocol::`
- [x] Public types re-exported from `nullslop-domain` crate root (`nullslop_domain::Command` works)
- [x] `cargo check -p nullslop-domain` passes
- [x] `cargo test -p nullslop-domain` passes (168 protocol tests)

---

## Review: Phase 2 — Move `nullslop-protocol` into `nullslop-domain/src/protocol/`

### Changes

- Copied all 59 files from `nullslop-protocol/src/` into `nullslop-domain/src/protocol/`
- Renamed `lib.rs` → `mod.rs` in the protocol module
- Bulk-replaced `crate::` → `crate::protocol::` across all moved files (111 occurrences)
- Added re-exports to domain crate root `lib.rs` for all public protocol types
- Fixed derive macro import (uses `nullslop_protocol_derive` directly, not through protocol module)

### Divergence Summary

- The `nullslop_protocol_derive` derive macros are imported directly in the crate root `lib.rs` rather than re-exported through the `protocol/` module. The original `protocol/mod.rs` still has the `pub use` line but the derive crate is an external dep, not a submodule.

### Verification

- `cargo check -p nullslop-domain` passes
- `cargo test -p nullslop-domain` passes (168 tests)
- No double-replacements in `crate::protocol::protocol::`

### Risks

- The `protocol/mod.rs` still has a `pub use nullslop_protocol_derive::{CommandMsg, EventMsg};` line which re-exports through the module. The domain root also re-exports these directly. This is harmless (re-exported from both places) but could be cleaned up.

### Next Steps

Phase 3: Merge `nullslop-component-ui` into `nullslop-domain/src/component_ui/`.
