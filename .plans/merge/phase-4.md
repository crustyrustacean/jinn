# Phase 4: Create `nsslice-tools`

Move tool orchestrator actor (1,475 lines, the largest single actor) from `actors/nullslop-tool-orchestrator` to `crates/slices/nsslice-tools`. No state migration.

## Steps

1. Create `crates/slices/nsslice-tools/` with Cargo.toml (same deps)
2. Copy actor code
3. Update root `Cargo.toml`, `src/app.rs`, e2e test imports
4. Delete `actors/nullslop-tool-orchestrator/`
5. Run `just check` then `just test`

## Acceptance Criteria

- [x] `nsslice-tools` crate exists at `crates/slices/nsslice-tools/` with the tool orchestrator code
- [x] `actors/nullslop-tool-orchestrator/` is deleted
- [x] `just check` passes
- [x] `just test` passes

---

## Review: Phase 4 — Create `nsslice-tools`

### Changes

- Created `crates/slices/nsslice-tools/` with the tool orchestrator actor (1,475 lines, identical logic)
- Updated root `Cargo.toml`, `src/app.rs`, and e2e test crate to use `nsslice_tools`
- Deleted `actors/nullslop-tool-orchestrator/` directory

### Divergence Summary

- Same e2e test update pattern as Phase 3.

### Verification

- `just check` — zero errors
- `just test` — all pass

### Risks

- None.

### Next Steps

Proceed to Phase 5: Absorb into `nsslice-provider`.
