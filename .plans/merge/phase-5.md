# Phase 5: Absorb into `nsslice-provider`

Move provider actor + LLM discover actor into the existing `nsslice-provider` slice. Create `nsslice-provider-protocol` for `ProviderState`.

## Steps

1. Create `crates/slices/nsslice-provider-protocol/` with Cargo.toml (deps: `nullslop-protocol`, `nullslop-providers`, `nullslop-selection-widget`, `jiff`)
2. Move `ProviderState` from `nullslop-component/src/app_state.rs` into `nsslice-provider-protocol/src/lib.rs`
3. Update `nullslop-component` to depend on `nsslice-provider-protocol`, import `ProviderState` from there
4. Remove `ProviderState` definition from `app_state.rs` (keep re-export from protocol crate if needed)
5. Add actor deps to `nsslice-provider/Cargo.toml`: `nullslop-actor`, `nullslop-services`, `wherror`, `error-stack`, `llm`, `tokio`, `tracing`
6. Move provider actor into `nsslice-provider/src/actor.rs`
7. Move LLM discover actor into `nsslice-provider/src/discover.rs`
8. Update `src/app.rs` imports
9. Delete `actors/nullslop-provider-actor/` and `actors/nullslop-llm-discover/`
10. Run `just check` then `just test`

## Acceptance Criteria

- [x] `nsslice-provider-protocol` exists with `ProviderState`
- [x] Provider actor and discover actor code lives in `nsslice-provider`
- [x] `nullslop-component` imports `ProviderState` from protocol crate
- [x] `actors/nullslop-provider-actor/` and `actors/nullslop-llm-discover/` are deleted
- [x] `just check` passes
- [x] `just test` passes

---

## Review: Phase 5 — Absorb into `nsslice-provider`

### Changes

- Created `nsslice-provider-protocol` with `ProviderState` (moved from `nullslop-component/src/app_state.rs`)
- Moved provider actor into `nsslice-provider/src/actor.rs` (369 lines)
- Moved LLM discover actor into `nsslice-provider/src/discover.rs` (184 lines)
- Updated `nullslop-component` to depend on and re-export `ProviderState` from the protocol crate
- Updated `src/app.rs` to import from `nsslice_provider::actor` and `nsslice_provider::discover`
- Deleted `actors/nullslop-provider-actor/` and `actors/nullslop-llm-discover/`

### Divergence Summary

- Had to use `pub use` for `ProviderState` in `app_state.rs` instead of a private `use` import, since both the re-export and the private import caused `E0252`.

### Verification

- `just check` — zero errors
- `just test` — all pass

### Risks

- None.

### Next Steps

Proceed to Phase 6: Absorb into `nsslice-session-management`.
