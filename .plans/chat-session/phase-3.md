# Phase 3: Remove old module and verify

## Problem

The `chat_session` module still exists in `nullslop-component` but all consumers now import from `nsslice-chat-session-protocol`. The old module is dead code that needs removal.

## What Moves / What Stays

**Deleted:**
- `crates/common/nullslop-component/src/chat_session/` directory (3 files: `mod.rs`, `state.rs`, `state_tests.rs`)

**No other changes.**

## File Changes

### 1. DELETE `crates/common/nullslop-component/src/chat_session/` directory

Contains:
- `mod.rs` — re-exports from state.rs
- `state.rs` — ChatSessionState, SessionCore, SessionUi
- `state_tests.rs` — 70 tests

All of this code now lives in `nsslice-chat-session-protocol`.

## Implementation Order

1. Delete the directory
2. Run `just check` to verify compilation
3. Run `just test` to verify all tests pass

## Acceptance Criteria

- [x] `crates/common/nullslop-component/src/chat_session/` directory no longer exists
- [x] `just check` passes
- [x] `just test` passes (1281/1281 unit tests + all e2e tests)

---

## Review: Phase 3 — Remove old module and verify

### Changes

Deleted `crates/common/nullslop-component/src/chat_session/` directory which contained `state.rs`, `state_tests.rs`, and a `state/` subdirectory with `queue.rs`, `scroll.rs`, `selection.rs`, `streaming.rs` (a recent refactoring of the original monolithic file into sub-modules). All of this code was already dead — `pub mod chat_session` was removed from `lib.rs` in Phase 2, and all consumers now import from `nsslice-chat-session-protocol`.

### Divergence Summary

The old `state.rs` had been refactored into sub-modules (`queue.rs`, `scroll.rs`, `selection.rs`, `streaming.rs`) since the initial plan was written. This didn't affect the extraction because the new crate was created from the original monolithic version and already contains all methods. The sub-modules were dead code at deletion time since `pub mod chat_session` was already removed.

### Verification

- `just check` — full workspace compiles
- `just test` — 1281/1281 unit tests pass + all e2e tests pass
- Verified the directory no longer exists

### Risks

None. All tests pass and the old module was already dead code.

### Next Steps

Task complete. All phases finished.
