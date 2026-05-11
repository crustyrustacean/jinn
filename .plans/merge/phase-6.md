# Phase 6: Absorb into `nsslice-session-management`

Move session actor + merge `nullslop-session` crate into the existing slice. **Deferred:** State migration (`ChatSessionState`, `SessionState` → protocol crate) blocked on `ChatInputBoxState` protocol crate (Phase 10).

## Steps

1. Copy `nullslop-session` crate files into `nsslice-session-management/src/persistence/` (persisted_session.rs, session_store.rs)
2. Move session actor from `actors/nullslop-session-actor/src/lib.rs` into `nsslice-session-management/src/actor.rs`
3. Add dependencies to `nsslice-session-management/Cargo.toml`: `nullslop-actor`, `nullslop-session` deps (serde, dirs, error-stack, wherror, etc.), tokio, tracing
4. Update `src/app.rs` imports
5. Delete `actors/nullslop-session-actor/` and `crates/nullslop-session/`
6. Remove both from root `Cargo.toml`
7. Run `just check` then `just test`

- [x] Session actor code lives in `nsslice-session-management/src/actor.rs`
- [x] Session persistence code lives in `nsslice-session-management-protocol`
- [x] `actors/nullslop-session-actor/` and `crates/nullslop-session/` are deleted
- [x] `just check` passes
- [x] `just test` passes

---

## Review: Phase 6 — Absorb into `nsslice-session-management`

### Changes

- Created `nsslice-session-management-protocol` with persistence types (`PersistedSession`, `SessionStore`, `JsonlSessionStore`, `SessionStoreService`, etc.)
- Moved session actor into `nsslice-session-management/src/actor.rs`
- Persistence module re-exports from protocol crate
- Updated all consumers: `nullslop-services`, `nullslop-component`, `src/session_conversion.rs`, e2e tests
- Deleted `actors/nullslop-session-actor/` and `crates/nullslop-session/`

### Divergence Summary

- **State migration deferred.** `ChatSessionState` and `SessionState` depend on `ChatInputBoxState`, which doesn't have its own protocol crate yet (Phase 10). Created `nsslice-session-management-protocol` with persistence types only, not state types.
- Updated the high-level plan to note the deferral.

### Verification

- `just check` — zero errors
- `just test` — all pass

### Risks

- State migration for `ChatSessionState`/`SessionState` must happen after Phase 10 creates `nsslice-chat-input-box-protocol`.

### Next Steps

Proceed to Phase 7: Create `nsslice-context`.
