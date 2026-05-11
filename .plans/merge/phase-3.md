# Phase 3: Create `nsslice-llm`

Move LLM streaming actor (1,263 lines) from `actors/nullslop-llm` to `crates/slices/nsslice-llm`. No state migration — just the actor move.

## Steps

1. Create `crates/slices/nsslice-llm/` with Cargo.toml (same deps as current actor)
2. Copy `actors/nullslop-llm/src/lib.rs` to `crates/slices/nsslice-llm/src/lib.rs`
3. Update root `Cargo.toml`: replace `nullslop-llm` with `nsslice-llm` in workspace members, deps, and workspace deps
4. Update `src/app.rs` imports
5. Delete `actors/nullslop-llm/`
6. Run `just check` then `just test`

## Acceptance Criteria

- [x] `nsslice-llm` crate exists at `crates/slices/nsslice-llm/` with the LLM actor code
- [x] `actors/nullslop-llm/` is deleted
- [x] `just check` passes
- [x] `just test` passes

---

## Review: Phase 3 — Create `nsslice-llm`

### Changes

- Created `crates/slices/nsslice-llm/` with the LLM actor code (1,263 lines, identical logic)
- Updated root `Cargo.toml`, `src/app.rs`, and `tests/nullslop-e2e/tests/actor.rs` to use `nsslice_llm`
- Deleted `actors/nullslop-llm/` directory

### Divergence Summary

- Had to also update `tests/nullslop-e2e/tests/actor.rs` — the plan didn't mention the e2e test crate but it imports `nullslop_llm` directly.

### Verification

- `just check` — zero errors
- `just test` — all pass

### Risks

- None. The LLM actor is self-contained with no state.

### Next Steps

Proceed to Phase 4: Create `nsslice-tools`.
