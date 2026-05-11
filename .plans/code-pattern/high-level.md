# Plan: Add Block Scoping Pattern to AGENTS.md

## Problem

The style guide doesn't express a preference for block scoping. The codebase has two patterns that leave unnecessary bindings in scope:

1. **Create-then-configure** — a mutable binding lives past its setup phase (e.g., `let mut registry = new(); configure(&mut registry);`)
2. **Intermediate values** — temporary bindings used only to compute a final value (e.g., `let a = 1; let b = 2; let c = a + b;`)

Both cases benefit from wrapping in a block expression, yielding an immutable binding and a tighter scope.

## Rationale

Block scoping reduces the number of variables floating around a function, makes the code easier to read, and makes it easier to extract into functions later. The user explicitly requested this pattern and provided concrete examples for both scenarios.

## Acceptance Criteria

- AGENTS.md has a new `### Block Scoping` subsection in section 2 (Core Patterns), placed after `### Dependency Injection`
- Both scenarios (create-then-configure and intermediate values) are covered with ❌ BAD / ✅ GOOD code examples
- The guidance states the rationale: fewer bindings in scope, easier to extract into functions later

## Implementation

- [x] Phase 1: Add "Block Scoping" subsection to AGENTS.md
  - [x] Add `### Block Scoping` after `### Dependency Injection` in section 2
  - [x] Document the create-then-configure scenario using the user's `AppUiRegistry` example
  - [x] Document the intermediate-values scenario using the user's `a + b` example
  - [x] State the shared rationale
