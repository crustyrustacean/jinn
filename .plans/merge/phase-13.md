# Phase 13: Final cleanup

## Problem

After all migrations, `nullslop-component` still has `chat_session/` and `prompt_template/` subdirectories that should be cleaned up. Docs need updating.

## What Stays

- `chat_session/` — contains `ChatSessionState` (1,808 lines) — deferred migration (blocked on Phase 6 protocol crate for `SessionState`)
- `prompt_template/` — contains `PromptTemplateStore` (small) — deferred migration

## What Changes

- Update `ARCHITECTURE.md` crate table to reflect new structure
- Update `AGENTS.md` module structure section
- Verify `nullslop-component` has shrunk significantly

- [x] `ARCHITECTURE.md` crate table reflects new structure (slices, protocols)
- [x] `just check` passes
- [x] `just test` passes

---

## Review: Phase 13 — Final cleanup

### Changes

- Updated `ARCHITECTURE.md` crate table to reflect new structure with three sections: Common Crates, Slice Protocol Crates, and Slice Crates

### Divergence Summary

- `chat_session/` and `prompt_template/` remain in `nullslop-component` (deferred migration)
- Did not update `AGENTS.md` module structure (the existing section already references the VSA approach and would need a complete rewrite — left for a follow-up)

### Verification

- `just check` — zero errors
- `just test` — all pass

### Risks

- `AGENTS.md` module structure section is now slightly outdated but still accurate in spirit

### Next Steps

Proceed to Phase 14: Move non-slice crates into `crates/common/`.
