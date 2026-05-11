# Phase 7: Create `nsslice-context`

Move context actor + prompt scan actor + merge `nullslop-context` crate into `nsslice-context`. Create `nsslice-context-protocol` for strategy types (used by `nullslop-services` and root crate). **Deferred:** `ContextAssemblyState` and `PromptTemplateStore` migration to protocol crate.

## Steps

1. Create `crates/slices/nsslice-context-protocol/` with strategy types from `nullslop-context`
2. Create `crates/slices/nsslice-context/` with actor code + remaining strategy impl code
3. Update `nullslop-services` and root crate to import from protocol crate
4. Update `src/app.rs` imports
5. Delete `actors/nullslop-context-actor/`, `actors/nullslop-prompt-scan/`, `crates/nullslop-context/`
6. Run `just check` then `just test`

- [x] `nsslice-context-protocol` exists with strategy types
- [x] `nsslice-context` exists with actor + strategy implementations
- [x] `nullslop-services` imports strategy types from protocol crate
- [x] Old actor/crate directories are deleted
- [x] `just check` passes
- [x] `just test` passes

---

## Review: Phase 7 — Create `nsslice-context`

### Changes

- Created `nsslice-context-protocol` with all prompt assembly strategy types (1,918 lines from `nullslop-context`)
- Created `nsslice-context` with context actor (1,380 lines) and prompt scan actor (108 lines)
- Updated `nullslop-services`, root crate, and e2e tests to import from protocol crate
- Deleted `actors/nullslop-context-actor/`, `actors/nullslop-prompt-scan/`, `crates/nullslop-context/`
- Removed `actors/*` glob from workspace members (actors directory is now empty and deleted)

### Divergence Summary

- `ContextAssemblyState` and `PromptTemplateStore` migration to protocol crate deferred (not blocking other phases).
- Phase 12 (Dissolve `actors/`) is now effectively complete — actors directory was deleted in this phase.

### Verification

- `just check` — zero errors
- `just test` — all pass

### Risks

- None.

### Next Steps

Proceed to Phase 8: Migrate `DashboardState`.
