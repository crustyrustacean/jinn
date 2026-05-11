# QA: Improve Agent Adherence in VSA-INSTRUCTIONS.md

## Problem

The autonomous agent running the `merge` plan has three adherence gaps:

1. **Shallow phase plans** — "detailed phase plan" in Step 2 is undefined, so the agent copies the high-level plan steps verbatim. Compare `merge/phase-5.md` (25 lines, no file manifest) with `vsa-2/phase-2.md` (276 lines, file-level manifest, code snippets, import mappings).
2. **Checklists not checked off** — The agent implements the work but never goes back to tick `[ ]` → `[x]` in its own phase plan or the high-level plan. Phase 5 is partially complete (commits exist, code moved) but all criteria remain `[ ]`.
3. **Cleanup skipped** — Old directories like `actors/nullslop-provider-actor/` persist after code moves because the agent never verifies deletions.

Root cause: `VSA-INSTRUCTIONS.md` defines *what* to produce but not the *standard of rigor*. The agent does the minimum because the instructions don't require more.

## Decisions

### Rename "detailed phase plan" → "execution plan"

The user suggested this. "Detailed" is subjective; "execution plan" implies a plan you can execute from without referring back to the high-level plan. The name sets the right expectation.

### Structure matches what already worked

The phase plans from `vsa`, `vsa-2`, `vsa-3`, and `vsa-5` all followed the same pattern naturally — Problem, What Moves/Stays, File Changes with code snippets, Implementation Order, Acceptance Criteria. This structure is being codified because it already proved effective. The agent produced these good plans in earlier sessions, so the structure is familiar territory.

### Use `vsa/phase-2.md` (status bar extraction) as the example

This is the right level of detail for a simple phase — 8 file changes, ~100 lines of plan, clear before/after code snippets. Not too long, not too short. The user confirmed they want a concrete example in the instructions.

### Update VSA-INSTRUCTIONS.md in place

The agent re-reads `VSA-INSTRUCTIONS.md` at Step 0/Step 1 of every loop iteration. Updating the file in place means the agent picks up changes automatically on the next loop — no special intervention needed. The running agent will re-read the updated instructions when it finishes the current sub-phase and loops back.

### No changes to the high-level plan or existing phase plans

Those are the running agent's working state. Changing them mid-flight would cause confusion. Only the instructions (the rules) are being updated.

## Acceptance Criteria

1. `VSA-INSTRUCTIONS.md` Step 2 defines an execution plan template with: Problem, What Moves/Stays, File Changes (with code snippets), Implementation Order, Acceptance Criteria
2. `VSA-INSTRUCTIONS.md` Step 4 includes a cleanup sweep sub-step requiring explicit `ls` verification of deletions
3. `VSA-INSTRUCTIONS.md` has a new "Verify acceptance criteria" step between test and commit, requiring `[ ]` → `[x]` with evidence
4. `VSA-INSTRUCTIONS.md` Step 8 requires re-reading the execution plan and confirming all criteria are `[x]` before marking the high-level phase complete
5. `VSA-INSTRUCTIONS.md` includes a concrete example execution plan based on the actual `vsa/phase-2.md` (status bar extraction)
6. The review template, definitions, end goal, and all other existing content are preserved unchanged

## Implementation Phases

- [x] **Phase 1: Patch VSA-INSTRUCTIONS.md**
  - [x] Update Step 2: rename to "execution plan", add required template structure (Problem, What Moves/Stays, File Changes with code snippets, Implementation Order, Acceptance Criteria)
  - [x] Update Step 4: add cleanup sweep sub-step after code changes, before `just check`
  - [x] Insert new Step between test (old Step 5) and commit (old Step 6): "Verify acceptance criteria" — re-read execution plan, check off every `[ ]` → `[x]` with evidence, fix gaps before proceeding
  - [x] Update Step 8 (renumbered): add checklist reconciliation — re-read execution plan, confirm all criteria are `[x]`, only then mark high-level phase as `[x]`
  - [x] Add example execution plan section (based on `vsa/phase-2.md` — status bar extraction, showing the right level of detail)
  - [x] Renumber all steps to account for the inserted step
  - [x] Verify all existing content (review template, definitions, end goal, example review) is preserved
