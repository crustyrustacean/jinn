# more-modules

## Problem

The `nullslop-protocol` crate (at `crates/common/nullslop-protocol/src/`) uses `mod.rs` files for all 13 of its submodules. The codebase convention is to use the Rust 2018+ style where `foo.rs` sits alongside `foo/` instead of `foo/mod.rs` inside it. This is a pure file-move refactoring — no logic changes.

## Current State

13 directories use `mod.rs`:

| Module | Subfiles | mod.rs nature |
|---|---|---|
| `actor/` | `command.rs`, `event.rs` | Declarative only |
| `chat_input/` | `command.rs`, `event.rs` | Declarative only |
| `context/` | `command.rs`, `event.rs`, `strategy_id.rs` | Declarative only |
| `context_strategy_picker/` | `entries.rs` | Declarative only |
| `custom/` | `command.rs`, `event.rs`, `derive_tests.rs` | Declarative only |
| `keymap_picker/` | `entries.rs` | Declarative only |
| `provider/` | `command.rs`, `convert.rs`, `event.rs`, `message.rs` | Declarative only |
| `provider_picker/` | `entries.rs` | Declarative only |
| `session/` | `session_load_completed.rs`, `session_load_requested.rs`, `session_new.rs`, `session_save_requested.rs` | **Has inline code** (`SessionId`, impls, tests) |
| `session_picker/` | `entries.rs` | Declarative only |
| `system/` | `command.rs`, `event.rs` | Declarative only |
| `tab/` | `active_tab.rs`, `command.rs` | **Has inline code** (`TabDirection`, Display impl) |
| `tool/` | `command.rs`, `event.rs`, `types.rs` | Declarative only |

Two modules — `session` and `tab` — have inline type definitions and tests in their `mod.rs`. All others are purely declarative (`mod` + `pub use`).

## Approach

For **declarative-only** modules: simply `mv foo/mod.rs foo.rs`. Rust resolves `foo.rs` + `foo/` the same way as `foo/mod.rs`. No code changes needed.

For **modules with inline code**: extract the inline types into dedicated files first (matching the "one type per file" convention used throughout the crate), then move the now-declarative `mod.rs` to `<name>.rs`.

- `session/mod.rs`: Extract `SessionId` struct, `impl SessionId`, `impl Default for SessionId`, and the `#[cfg(test)] mod tests` block into `session/session_id.rs`. The remaining `mod.rs` becomes purely declarative (`mod session_id;` + `pub use` + `pub mod` for existing submodules).
- `tab/mod.rs`: Extract `TabDirection` enum and `impl Display for TabDirection` into `tab/tab_direction.rs`. The remaining `mod.rs` becomes purely declarative.

## Plan

- [x] Phase 1: Convert declarative-only modules (move `mod.rs` → `<name>.rs`)
  - [x] `actor/mod.rs` → `actor.rs`
  - [x] `chat_input/mod.rs` → `chat_input.rs`
  - [x] `context/mod.rs` → `context.rs`
  - [x] `context_strategy_picker/mod.rs` → `context_strategy_picker.rs`
  - [x] `custom/mod.rs` → `custom.rs`
  - [x] `keymap_picker/mod.rs` → `keymap_picker.rs`
  - [x] `provider/mod.rs` → `provider.rs`
  - [x] `provider_picker/mod.rs` → `provider_picker.rs`
  - [x] `session_picker/mod.rs` → `session_picker.rs`
  - [x] `system/mod.rs` → `system.rs`
  - [x] `tool/mod.rs` → `tool.rs`
  - [x] Verify with `just check`

- [x] Phase 2: Extract inline code from `session/mod.rs` and convert
  - [x] Create `session/session_id.rs` containing `SessionId` struct, `impl SessionId`, `impl Default`, and the `#[cfg(test)] mod tests` block
  - [x] Rewrite `session/mod.rs` to be purely declarative: add `mod session_id;` and `pub use session_id::SessionId;`
  - [x] Move `session/mod.rs` → `session.rs`
  - [x] Verify with `just check`

- [x] Phase 3: Extract inline code from `tab/mod.rs` and convert
  - [x] Create `tab/tab_direction.rs` containing `TabDirection` enum and `impl Display`
  - [x] Rewrite `tab/mod.rs` to be purely declarative: add `mod tab_direction;` and `pub use tab_direction::TabDirection;`
  - [x] Move `tab/mod.rs` → `tab.rs`
  - [x] Verify with `just check`

- [x] Phase 4: Final validation
  - [x] Confirm no `mod.rs` files remain under `crates/common/nullslop-protocol/src/`
  - [x] Run `just test` — all tests pass
  - [x] Run `just lint` — pre-existing lint failures in e2e tests unrelated to this change. Protocol crate passes clean.

## Acceptance Criteria

- [x] Zero `mod.rs` files exist under `crates/common/nullslop-protocol/src/`
- [x] Every former `foo/mod.rs` is now `foo.rs` (sibling to the `foo/` directory)
- [x] `just check` passes
- [x] `just test` passes
- [x] `just lint` — pre-existing failures unrelated to this change; protocol crate passes clean
