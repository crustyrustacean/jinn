# Phase 4: Split `nsslice-session-management/src/actor.rs`

## Problem

`actor.rs` was 1494 lines containing the actor struct, Actor impl, dispatchers, persistence handlers, command handlers, event handlers, and ~700 lines of tests.

## What Moved / What Stays

**Moved to `src/actor/handlers/`:**
- `command.rs` (184 lines): `EnqueueAction` enum + all command handler methods
- `event.rs` (96 lines): all event handler methods (stream, tool call)
- `persistence.rs` (93 lines): save/load handlers

**`src/actor/mod.rs` (153 production + 1014 test = 1167 lines):**
- Struct definitions + `Actor` impl + `handle_event()`/`handle_command()` dispatchers + all tests

## Acceptance Criteria

- [x] `src/actor/` directory exists with `mod.rs` and `handlers/` submodule
- [x] `src/actor.rs` is deleted
- [x] `just check` passes
- [x] `just test` passes
- [x] `just lint` passes

---

## Review: Phase 4 — Split nsslice-session-management actor.rs

### Changes

Restructured `actor.rs` into `actor/mod.rs` + `actor/handlers/{command,event,persistence}.rs`. Handler methods remain as `impl SessionPersistenceActor` blocks in their respective files, accessed via `pub(in crate::actor)` visibility.

### Divergence Summary

- Used `pub(in crate::actor)` instead of `pub(super)` for handler method visibility. The handler files are two levels deep (`actor/handlers/command.rs`), so `pub(super)` would only reach `handlers/`, not `actor/mod.rs`. `pub(in crate::actor)` correctly scopes visibility to the `actor` module and all its descendants.

### Verification

- `just check` — clean
- `just test` — all pass
- `just lint` — pass

### Risks

None.

### Next Steps

Phase 5: Split `nsslice-context/src/actor.rs` (same pattern).
