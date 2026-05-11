# Phase 1: Create `nsslice-echo`

Move the echo actor from `actors/nullslop-echo` to `crates/slices/nsslice-echo`. This is the simplest actor — no state migration, no protocol crate needed. Proves the actor-to-slice pattern before touching complex actors.

## Steps

1. Create `crates/slices/nsslice-echo/Cargo.toml` — copy dependencies from `actors/nullslop-echo/Cargo.toml`, change name to `nsslice-echo`
2. Create `crates/slices/nsslice-echo/src/lib.rs` — copy contents from `actors/nullslop-echo/src/lib.rs` verbatim
3. Add `nsslice-echo` to root `Cargo.toml`:
   - Add to `members` array in `[workspace]`
   - Add to `[workspace.dependencies]` with `path = "crates/slices/nsslice-echo"`
   - Replace `nullslop-echo` dependency in `[dependencies]` with `nsslice-echo`
   - Remove `nullslop-echo` from `[workspace.dependencies]`
4. Update `src/app.rs` imports: `nullslop_echo` → `nsslice_echo`
5. Remove `actors/nullslop-echo` from workspace members (remove `actors/*` glob won't work since other actors remain — the glob covers all, so nothing to change there)
6. Delete `actors/nullslop-echo/` directory
7. Run `just check` then `just test`

## Acceptance Criteria

- [x] `nsslice-echo` crate exists at `crates/slices/nsslice-echo/` with the echo actor code
- [x] `actors/nullslop-echo/` directory is deleted
- [x] Root `Cargo.toml` lists `nsslice-echo` in workspace members, workspace deps, and `[dependencies]`
- [x] `nullslop-echo` is removed from root `Cargo.toml` workspace deps and `[dependencies]`
- [x] `src/app.rs` imports from `nsslice_echo` instead of `nullslop_echo`
- [x] `just check` passes with zero errors
- [x] `just test` passes — no regressions

---

## Review: Phase 1 — Create `nsslice-echo`

### Changes

- Created `crates/slices/nsslice-echo/` with the echo actor code (identical logic, updated module doc)
- Updated root `Cargo.toml`: added `nsslice-echo` to workspace members, workspace deps, and `[dependencies]`; removed `nullslop-echo` from workspace deps and `[dependencies]`
- Updated `src/app.rs`: replaced all `nullslop_echo` references with `nsslice_echo`
- Deleted `actors/nullslop-echo/` directory

### Divergence Summary

- Fossil database was read-only in this environment; commit could not be completed. All code changes are correct and tested.

### Verification

- `just check` — zero errors
- `just test` — all pass (including e2e cucumber tests)
- Verified `actors/nullslop-echo/` is deleted and `crates/slices/nsslice-echo/` exists

### Risks

- None. The echo actor is a standalone example actor with no state and no dependents.

### Next Steps

Proceed to Phase 2: Create `nsslice-shutdown` (shutdown tracker actor + first protocol crate).
