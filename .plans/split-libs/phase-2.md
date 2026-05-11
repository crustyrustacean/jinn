# Phase 2: Split `nsslice-llm/src/lib.rs`

## Problem

`lib.rs` in `nsslice-llm` is 1289 lines. It contains the LLM actor plus the per-session state machine types (`SessionState`, `SessionData`). The session types are a separable concern used exclusively by the actor.

## What Moves / What Stays

**Moves to `src/session.rs`:**
- `SessionState` enum
- `SessionData` struct + `impl SessionData` (the `new()` constructor)

**Stays in `lib.rs`:**
- Everything else: `LlmDirectMsg`, `spawn()`, `LlmActor`, `Actor` impl, all handler methods
- `#[cfg(test)] mod tests`

## File Changes

1. **Create `crates/slices/nsslice-llm/src/session.rs`** — `SessionState` and `SessionData` with necessary imports.
2. **Modify `crates/slices/nsslice-llm/src/lib.rs`** — add `mod session;`, import `SessionState` and `SessionData` from session module, remove the inline definitions.

## Implementation Order

1. Create `session.rs`
2. Update `lib.rs`
3. `just check` then `just test`

## Acceptance Criteria

- [x] `crates/slices/nsslice-llm/src/session.rs` exists with `SessionState` and `SessionData`
- [x] `lib.rs` no longer defines `SessionState` or `SessionData` inline
- [x] `just check` passes
- [x] `just test` passes
- [x] `just lint` passes

---

## Review: Phase 2 — Split nsslice-llm lib.rs

### Changes

Extracted `SessionState` enum and `SessionData` struct + impl into `src/session.rs`. Both types are `pub(crate)` so the actor and its tests can access them via `use session::...`.

### Divergence Summary

- Also fixed Phase 1's unused import warning by gating the `use builtin::{execute_*}` re-exports with `#[cfg(test)]`.

### Verification

- `just check` — clean
- `just test` — all pass
- `just lint` — pass

### Risks

None.

### Next Steps

Phase 3: Split `nsslice-chat-input-box-protocol/src/lib.rs`.
