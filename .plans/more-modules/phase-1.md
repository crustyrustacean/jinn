# Phase 1: Convert declarative-only modules (move `mod.rs` → `<name>.rs`)

## Problem

11 of the 13 submodules in `nullslop-protocol` have `mod.rs` files that are purely declarative — they only contain `mod` and `pub use` statements with no inline types, impls, or tests. These can be converted by simply moving `foo/mod.rs` → `foo.rs` (the Rust 2018+ convention). No code changes required.

The remaining 2 modules (`session`, `tab`) have inline code and are deferred to later phases.

## What Moves

Each `foo/mod.rs` is moved to become `foo.rs` (sibling to the `foo/` directory):

| # | Source | Destination |
|---|--------|-------------|
| 1 | `src/actor/mod.rs` | `src/actor.rs` |
| 2 | `src/chat_input/mod.rs` | `src/chat_input.rs` |
| 3 | `src/context/mod.rs` | `src/context.rs` |
| 4 | `src/context_strategy_picker/mod.rs` | `src/context_strategy_picker.rs` |
| 5 | `src/custom/mod.rs` | `src/custom.rs` |
| 6 | `src/keymap_picker/mod.rs` | `src/keymap_picker.rs` |
| 7 | `src/provider/mod.rs` | `src/provider.rs` |
| 8 | `src/provider_picker/mod.rs` | `src/provider_picker.rs` |
| 9 | `src/session_picker/mod.rs` | `src/session_picker.rs` |
| 10 | `src/system/mod.rs` | `src/system.rs` |
| 11 | `src/tool/mod.rs` | `src/tool.rs` |

(All paths relative to `crates/common/nullslop-protocol/`.)

## What Stays

- `lib.rs` — no changes needed. It already declares `pub mod foo;` which works with both `foo/mod.rs` and `foo.rs`.
- All subfiles (`command.rs`, `event.rs`, etc.) inside each directory — untouched.
- `session/mod.rs` and `tab/mod.rs` — deferred to phases 2 and 3.
- All other top-level `.rs` files — untouched.

## Implementation Order

1. Move all 11 `mod.rs` files using `mv`.
2. Run `just check` to verify compilation.

## Acceptance Criteria

- [x] `src/actor/mod.rs` no longer exists, `src/actor.rs` exists
- [x] `src/chat_input/mod.rs` no longer exists, `src/chat_input.rs` exists
- [x] `src/context/mod.rs` no longer exists, `src/context.rs` exists
- [x] `src/context_strategy_picker/mod.rs` no longer exists, `src/context_strategy_picker.rs` exists
- [x] `src/custom/mod.rs` no longer exists, `src/custom.rs` exists
- [x] `src/keymap_picker/mod.rs` no longer exists, `src/keymap_picker.rs` exists
- [x] `src/provider/mod.rs` no longer exists, `src/provider.rs` exists
- [x] `src/provider_picker/mod.rs` no longer exists, `src/provider_picker.rs` exists
- [x] `src/session_picker/mod.rs` no longer exists, `src/session_picker.rs` exists
- [x] `src/system/mod.rs` no longer exists, `src/system.rs` exists
- [x] `src/tool/mod.rs` no longer exists, `src/tool.rs` exists
- [x] `just check` passes

---

## Review: Phase 1 — Convert declarative-only modules

### Changes

Moved 11 `mod.rs` files to become named module files (`foo.rs`) alongside their `foo/` directories. No code changes — pure file renames.

### Divergence Summary

None. All 11 moves completed as planned.

### Verification

- Verified all 11 `mod.rs` files are gone and 11 `.rs` files exist in their place.
- `just check` passed (full workspace compilation clean).

### Risks

None. This is a no-op from Rust's perspective — `foo.rs` + `foo/` resolves identically to `foo/mod.rs`.

### Next Steps

Phase 2: Extract inline code from `session/mod.rs` and convert it.
