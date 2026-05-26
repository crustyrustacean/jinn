# Mutant Kill Plan — mutant-215-350

## Problem

96 mutation testing survivors in `nullslop-domain` (Tiers 1–3) are not killed by existing tests.

## Solution

Add targeted tests to kill survivors, organized by tier priority.

## Phases

- [ ] Phase 1 — Tier 1: Core Routing & Dispatch (18 mutants)
- [ ] Phase 2 — Tier 2: Chat Input & Navigation (30 mutants)
- [ ] Phase 3 — Tier 3: Infrastructure & Lifecycle (26 mutants)

## Acceptance Criteria

- [ ] All 74 target mutants killed
- [ ] No production code changes — only test additions
- [ ] Given/When/Then comment pattern
- [ ] `cargo test` passes with zero regressions
