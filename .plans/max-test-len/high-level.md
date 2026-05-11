# max-test-len

## Problem

Inline test modules (`#[cfg(test)] mod tests { ... }`) that grow beyond 200 lines hurt readability. We need a linter that scans the codebase, detects oversized inline test modules, and reports them as warnings.

## Acceptance Criteria

- `just lint-testlength` scans all `.rs` files in the workspace (excluding `vendor/`)
- Detects inline test modules only (not external `mod tests;`)
- Reports any inline test module exceeding 200 lines as `WARN: file:line: module is N lines (max 200)`
- Exits 0 always (warning-only, not a gate)

## Decisions

- **Threshold**: 200 lines inclusive (counts from `#[cfg(test)]` line through closing `}`)
- **Error behavior**: Warning only — exits 0 always. Not wired into `just lint` or `just ci`.
- **Scope**: Excludes `vendor/` directory
- **Recipe name**: `just lint-testlength`
- **Implementation**: Python script inlined in the justfile recipe (same pattern as `apply-license`)
- **Output format**: `WARN: file:line: module is N lines (max 200)` — compiler-warning style

## Phases

- [ ] Phase 1: Add `lint-testlength` recipe to `justfile`
  - [ ] Write Python script inline in the justfile recipe that:
    - Walks all `.rs` files under the workspace root, skipping `vendor/`
    - Finds `#[cfg(test)]` followed by `mod name {` (inline only — `{` on the next non-empty line, not `;`)
    - Tracks brace depth to find the closing `}` and counts lines (inclusive of `#[cfg(test)]` and closing `}`)
    - Prints `WARN: file:line: module is N lines (max 200)` for each offender, using paths relative to workspace root
    - Exits 0 unconditionally
  - [ ] Run `just lint-testlength` and confirm the 27 known offenders are reported
