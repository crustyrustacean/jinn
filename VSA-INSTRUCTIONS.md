## Instructions

You are an autonomous coding agent specializing in Rust programming and have been tasked with migrating this codebase to a vertical slice architecture.

### Workflow Summary

0. **Read required files** (`AGENTS.md`, `ARCHITECTURE.md`, `justfile`, high-level plan)
1. **Find the next unchecked phase** in the high-level plan
2. **Write an execution plan** (Problem, What Moves/Stays, File Changes with code snippets, Implementation Order, Acceptance Criteria)
3. **Review the plan** against the checklist (conventions, circular deps, slice deps, consumers, tests, security)
4. **Implement** — step by step, `just check` after each group, cleanup sweep at the end
5. **Run `just test`** — full workspace suite must pass
6. **Verify acceptance criteria** — re-read execution plan, check off every `[ ]` → `[x]` with evidence
7. **Commit** with `fossil commit -m "<TASK> Phase N: <description>"`
8. **Write the phase review** — append to execution plan file
9. **Update the high-level plan** — checklist reconciliation first, then check off + apply divergences
10. **Loop** back to step 0

---

### Step 0: Required reading

Before touching any code, read these files in order:

1. **`AGENTS.md`** — Coding conventions, error handling, testing patterns, module structure, trait design rules.
2. **`ARCHITECTURE.md`** — Data flow, actor system, state ownership, communication channels.
3. **`justfile`** — Available build/test commands. Always prefer `just` commands over raw `cargo` invocations.
4. **`.plans/<task>/high-level.md`** — The high-level plan for the current task. `<task>` is the plan directory name (e.g., `vsa-6`).

You must understand these before proceeding. If anything is unclear, note it in the execution plan's "Risks" section.

### Step 1: Determine the next phase

Read `.plans/<task>/high-level.md` and find the first unchecked phase.

- If all phases are checked off → **stop**. The task is complete.
- If the next phase exists → proceed to step 2.

### Step 2: Create an execution plan

An **execution plan** is a self-contained document that you can implement from without referring back to the high-level plan. It must be detailed enough that each step maps to a specific file change with before/after code.

Save it to `.plans/<task>/phase-N.md` where `N` is the phase number and `<task>` matches the high-level plan directory.

**Required structure:**

1. **Problem** — What this phase solves and why it's needed.
2. **What Moves / What Stays** — Explicit scope boundary. List every type, function, test, and module that moves, and what remains unchanged.
3. **File Changes** — Numbered list of every file affected. For each file, include:
   - Whether it's being created, modified, or deleted.
   - The exact code change — show the before/after with snippets (imports, function signatures, Cargo.toml entries, etc.).
   - For moved code: the source path → destination path.
4. **Implementation Order** — The sequence to execute the file changes in.
5. **Acceptance Criteria** — A `[ ]` checkbox list at the bottom. Each item must be independently verifiable (a file exists, a directory is gone, a test passes, an import resolves).

**Do not copy-paste the high-level plan.** The high-level plan says *what* to achieve. The execution plan says *exactly how* — file by file, line by line. If you find yourself restating the high-level plan's steps, you're not writing an execution plan.

See the **Example Execution Plan** section at the bottom of this file for a concrete example of the right level of detail.

### Step 3: Review the execution plan

Before implementing, review the plan against this checklist:

- [ ] Does the plan follow the conventions in `AGENTS.md`? (error handling, testing patterns, module structure, trait design)
- [ ] Does the plan avoid introducing circular dependencies?
- [ ] Does the plan avoid introducing new slice-to-slice dependencies? (Only `-protocol` crates may be shared between slices.)
- [ ] Does the plan update all consumers when moving code or changing public APIs?
- [ ] Does the plan preserve all existing tests (or explicitly move them)?
- [ ] Are there security or performance implications?

If any checklist item fails, update the plan to address it before proceeding.

### Step 4: Implement the execution plan

Implement the plan step by step. After each logical group of changes, run `just check` to catch compile errors early.

If `just check` or `just test` fails:

- **Do NOT commit broken code.**
- Fix the issue, re-run `just check` / `just test`.
- If the fix requires changes to the plan, update the execution plan file first.
- If the fix reveals a fundamental problem with the plan (e.g., wrong approach, missing dependency), stop and append a note to the execution plan explaining the blocker.

**Cleanup sweep:** After all code changes are made and before running `just check`, verify every deletion listed in the execution plan. For each directory or file that should have been deleted, run `ls` to confirm it no longer exists. If it still exists, delete it and note the deletion in the review.

### Step 5: Run full verification

Run `just test` (the full workspace test suite). All tests must pass before committing.

### Step 6: Verify acceptance criteria

**This step is mandatory. Do not skip it.**

Re-read the execution plan file (`.plans/<task>/phase-N.md`). For every `[ ]` item in the Acceptance Criteria section:

1. Verify the criterion is met (run a command, check a file exists, confirm a directory is gone, etc.).
2. Change `[ ]` to `[x]` in the execution plan file.

If any criterion cannot be checked off, **stop and fix the gap** before proceeding to commit. Common gaps:
- A directory that should have been deleted still exists.
- An import still points to the old location.
- A test was not moved and is now missing.

### Step 7: Commit

```bash
fossil commit -m "<TASK> Phase N: <brief description>"
```

Example: `fossil commit -m "VSA-6 Phase 1: move provider actor into nsslice-provider"`

Use the task name, phase number, and a brief description of what changed.

### Step 8: Write the phase review

Append the review to the execution plan file (`.plans/<task>/phase-N.md`) using this template:

```markdown
---

## Review: Phase N — <phase name>

### Changes

A brief description of what changed and why.

### Divergence Summary

List things that did NOT go according to plan — changes, additions, omissions, or reordering.
If everything was implemented as planned, just say "None."

### Verification

What you did to verify the changes work (`just check`, `just test`, manual checks, etc.).

### Risks

Any concerns, things that might break, or follow-up work needed.

### Next Steps

Plan phase <N+1>.
```

### Step 9: Update the high-level plan

This is critical for continuity — the next agent depends on an accurate high-level plan.

**Checklist reconciliation — do this first:**

1. Re-read the execution plan file (`.plans/<task>/phase-N.md`).
2. Confirm every acceptance criterion is `[x]`. If any remain `[ ]`, do NOT mark the high-level phase as complete — go back and fix the gap.
3. Only after all criteria are verified, proceed to update the high-level plan.

**Then update the high-level plan:**

1. **Re-read** `.plans/<task>/high-level.md` to understand the full context.
2. **Check off** completed items using `[x]`.
3. **Apply divergences** — if the review found that future phases need a different approach:
   - Use markdown strikethrough (`~~text~~`) to mark removed or changed items.
   - Add a brief note at the end of the impacted section explaining what changed and why.
   - Example: if an unplanned change in phase 2 requires a different technique in phase 3, update phase 3 in the high-level plan so the next agent uses the correct approach.
4. **Split phases if needed** — if a remaining phase looks too large, break it into sequential phases (e.g., insert a new phase between existing ones). Use the next integer (e.g., if phase 3 needs splitting, insert phase 4 and renumber the old phase 4 to phase 5).

### Step 10: Loop

Return to step 1. When all phases in the high-level plan are checked off and `just test` passes, **stop**. Do not start work that isn't in the plan.

## Goal of this coding loop

- Migrate the codebase to a VSA (Vertical Slice Architecture)

### Definitions

- A "slice" is a single small to medium sized feature colocated into a single crate or module.
- Slices are meant to be one domain unit. If a developer wanted to make a change to any arbitrary thing, the first word they think of should probably be the slice.
  - Example: "I want to update the appearance of a user message" -> chat log -> chat entries. "Chat log" is the slice
- Slices can have subslices.
- Slices are NOT large integration components.
- Slices are DOMAIN concepts. They don't have to be user-facing.

Examples:

- status bar -> slice
- clock in status bar -> subslice
- the TUI -> not a slice (it's a host where other slices integrate)
- the actor system -> not a slice (it's infrastructure that runs the actors)
- an actor -> slice or part of a slice, depending on the domain
- appstate -> not a slice (it's a giant shared data structure)

Rule of thumb: If it's just data, then its probably not a slice. If it's only purpose is to host other things, it's probably _not_ a slice. User-facing interface things are almost always a slice.

## Eventual End Goal

We eventually want this directory structure:

```
crates/
  common/        <- shared crates here. these MUST NOT depend on a slice, but they CAN depend on a slice _protocol_!
  slices/        <- vertical slices here. these MUST NOT depend on eachother. EXCEPTION: `-protocol` crates are designed to be added as dependencies to anything
    nsslice-foo            <- the actual slice
    nsslice-foo-protocol   <- shared stuff required to communicate with the slice
```

- Dependency direction is unidirectional so the top-level app can depend on slices which depend on protocols, either of which can depend on common.

## Example Execution Plan

This is a real execution plan from a completed phase. It shows the level of detail expected — every file change has a source → destination, code snippets show the exact changes, and the acceptance criteria are independently verifiable.

```markdown
# Phase 2: Create `nsslice-status-bar` crate (rendering-only)

## Problem

This is the first real slice. The status bar is a pure rendering component — it has no intents, no validators, and no handlers. It reads `AppState` and draws text. This makes it the ideal candidate for the first migration: the pattern is simple (move code + wire up) and the risk is low.

The status bar currently lives in `nullslop-component/src/status_bar/` as two files: `mod.rs` (re-exports) and `element.rs` (the `StatusBarElement` struct, its `UiElement` impl, and all its tests).

## What Moves

- `StatusBarElement` struct + `UiElement` impl + 9 tests → `nsslice-status-bar/src/element.rs`
- Registration logic (currently in `nullslop-component::register_tui_elements()`) → `nsslice-status-bar::register()`

## What Stays

- All other elements in `nullslop-component` (chat log, chat input box, etc.)
- `AppState`, `FrontendState`, `State` wrapper — stay in `nullslop-component`

## File Changes

### 1. NEW `crates/slices/nsslice-status-bar/Cargo.toml`

```toml
[package]
name = "nsslice-status-bar"
version = "0.1.0"
edition = "2024"

[dependencies]
nullslop-component-ui = { workspace = true }
nullslop-component = { workspace = true }
nullslop-providers = { workspace = true }
ratatui = { workspace = true }

[dev-dependencies]
rstest = { workspace = true }
nullslop-protocol = { workspace = true }

[lints]
workspace = true
```

### 2. NEW `crates/slices/nsslice-status-bar/src/lib.rs`

```rust
//! Status bar slice — displays the active prompt strategy and current model.

pub mod element;
pub use element::StatusBarElement;

use nullslop_component::AppUiRegistry;

pub fn register(registry: &mut AppUiRegistry) {
    registry.register(Box::new(StatusBarElement));
}
```

### 3. NEW `crates/slices/nsslice-status-bar/src/element.rs`

Move from `nullslop-component/src/status_bar/element.rs`. Update imports:

```rust
// Before:
use crate::AppState;
// After:
use nullslop_component::AppState;
```

In test module:
```rust
// Before:
use crate::{AppState, ProviderState};
// After:
use nullslop_component::{AppState, ProviderState};
```

### 4. MODIFY root `Cargo.toml`

Add to `[workspace.dependencies]`:
```toml
nsslice-status-bar = { path = "crates/slices/nsslice-status-bar" }
```

Add to `[dependencies]`:
```toml
nsslice-status-bar = { workspace = true }
```

### 5. MODIFY `crates/nullslop-component/src/lib.rs`

Remove:
```rust
pub mod status_bar;
```

Remove from `register_tui_elements()`:
```rust
registry.register(Box::new(status_bar::StatusBarElement));
```

### 6. MODIFY `src/app.rs`

Add after both `nullslop_component::register_tui_elements()` and `register_all()` calls:
```rust
nsslice_status_bar::register(&mut ui_registry);
```

### 7. DELETE `crates/nullslop-component/src/status_bar/` directory

## Implementation Order

1 → 2 → 3 → 4 → 5 → 6 → 7 → `just check` → `just test`

## Acceptance Criteria

- [ ] `crates/slices/nsslice-status-bar/` exists with `Cargo.toml`, `src/lib.rs`, `src/element.rs`
- [ ] `nsslice-status-bar` is in workspace dependencies
- [ ] `nullslop-component` no longer has `pub mod status_bar;`
- [ ] `nullslop-component::register_tui_elements()` no longer registers `StatusBarElement`
- [ ] Binary crate depends on `nsslice-status-bar` and calls `nsslice_status_bar::register()`
- [ ] All status bar tests (9 tests) pass from the new crate
- [ ] `just check` succeeds with zero errors
- [ ] `just test` succeeds — all tests pass
```

Notice how every file change has before/after code, every moved item has a source → destination, and the acceptance criteria are each independently verifiable. This is the standard.

## Example Review: Phase 1 — Dependency Upgrade

### Changes

- Bumped eframe/egui 0.31 → 0.34, added egui_taffy, taffy, log
- Updated `update()` → `ui()` to match eframe 0.34 trait change, changed `ctx` refs to `ui.ctx()`, simplified `on_exit()` signature
- Added `source_size` field to ColorImage construction (new required field in 0.34)

### Divergence Summary

- `on_exit()` signature change was not in the plan — eframe 0.34 removed the glow context parameter
- Added `log` workspace dependency — plan didn't mention it but egui_taffy requires it

### Verification

- `just check` — zero errors
- `just test` — all pass
- Deprecation warnings for TopBottomPanel, SidePanel are expected (Phase 2 replaces these)

### Risks

- Deprecation warnings mean these APIs may be removed in a future egui version
- Phase 2 must account for the extra API changes found here

### Next Steps

This was the last phase. Implementation complete.
