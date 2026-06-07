---
name: phased-task-loop
description: Structured phased implementation workflow for multi-phase coding tasks. Use when the user wants to execute a phased plan with execution plans, acceptance criteria verification, phase reviews, and spec reconciliation. Agents MUST continue through all phases autonomously without stopping between phases.
---

# Phased Task Loop

A disciplined workflow for implementing multi-phase coding tasks. The task list tracks progress in real time.

**Project commands are referenced by role** (`check`, `test`, `lint`, `format`, `commit`, `sync-trunk`, `vcs`). The project's `AGENTS.md` resolves each role to its actual command (e.g. for jinn: `test` → `just test`, `commit` → `fossil commit`, `sync-trunk` → `fossil merge trunk`). When this skill says "run the project's `test` command," look up `test` in `AGENTS.md` and run that. If the project does not define a role, skip it.


---

## Constraints

1.  **Branch only.** Stay on your branch. Never commit to the project's main line.
2.  **Sync before next phase.** Run the project's `sync-trunk` command before moving to the next phase.
3.  **No reverse merge.** Never merge your branch onto the project's main line.
4.  **Continuous execution.** Proceed from one phase to the next without stopping. Only stop when all phases are complete or an unrecoverable error blocks progress.
5.  **Stay in `.plans/<task>/`.** All execution plans go here. Do not create new directories.
6.  **Never rewrite the spec.** The spec (`plan.md`) is immutable — annotate only (strikethrough, divergence notes). The task list tracks status, not checkboxes in the spec.
7.  **One task per turn.** Each assistant turn advances exactly one task and ends with `todo_complete_task`. If a task grew beyond a single turn of work, you went too deep — split it via `todo_add_task` and pick up the new sub-task next turn.

---

## Concepts

**Task list** — Live progress tracker, managed via `todo_*` tool calls. Update immediately when state changes (complete, cancel, postpone, add).

**Spec** — The file `plan.md`. Immutable reference; annotate only.

**Execution plan** — The file `phase-N.md`. Contains file-by-file implementation details and a verification checklist (acceptance criteria) at the bottom using `[ ]`/`[x]`.

**Task list ≠ acceptance criteria.** The task list tracks _what's done_. Acceptance criteria verify _correctness_. They are separate.

**NEXT block** — Every `todo_*` tool call returns a `→ NEXT` block at the top of its result. It names the next task you should work on (or tells you the phase is complete / all phases are complete). Read it. Obey it. The block exists because the model otherwise drifts.

**Phase-aware list** — Every `todo_*` tool result includes the full task list, with phases you are not currently working on prefixed `(Blocked by previous phase)`. This is a cue, not a hard lock — you cannot jump to a blocked phase until the current one finishes.

---

## Task List Discipline

Update the task list **at the moment a decision is made**, never retroactively:

- Task's work is done → `todo_complete_task` immediately (during implementation, not batched at phase end)
- Discovered unplanned work → `todo_add_task` or `todo_add_phase` right away
- Task no longer needed → `todo_cancel_task` (shows as "CANCELLED: \<description\>")
- Task belongs in a different phase → `todo_postpone_task` or `todo_postpone_to_phase`

**Do not batch.** A pattern of "do five things, then call `todo_complete_task` five times" is the failure mode this skill exists to prevent. One task, one completion call, repeat.

---

## The Loop

**Repeat the following cycle until all tasks are complete.** Work per-phase, top to bottom.

1.  **Check status.** Call `todo_get_task_list`. If all tasks in all phases are complete → **done, stop.** Otherwise, the NEXT block at the top of the result names the next task to work on. Begin there.

2.  **Write an execution plan.** Save to `.plans/<task>/phase-N.md`. Required sections:
    - **Problem** — What and why.
    - **What Moves / What Stays** — Scope boundary.
    - **File Changes** — Numbered list. For each: created/modified/deleted, before/after code snippets, source→destination for moves.
    - **Implementation Order** — Sequence of file changes.
    - **Acceptance Criteria** — `[ ]` checklist. Each item independently verifiable (file exists, test passes, import resolves).

    Do not copy-paste the spec. The execution plan says _how_, file by file.

    Before implementing, check: follows conventions? No circular deps? All consumers updated? Tests preserved?

3.  **Implement the current task only.** This is a sub-loop, run once per task:

    a. **Restate** the current task in one sentence before touching the keyboard.

    b. **Do the work** for that one task. Run the build command after each logical group of changes.

    c. **Complete the task.** Call `todo_complete_task` for the task you just finished.

    d. **Read the NEXT block** returned by `todo_complete_task`. It points at the next task (or says "phase complete — proceed to verify", or "all phases complete — stop").

    e. **Repeat from (a)** with the task named by the NEXT block, until the NEXT block says the phase is complete.

    If the build fails: fix, re-run, continue. Do not commit broken code. For truly unrecoverable blockers: document in the execution plan and stop.

    If you discover untracked work: call `todo_add_task` / `todo_add_phase` immediately — then resume the sub-loop at step (a) with whichever task is now next per the NEXT block.

    If a task grew beyond one assistant turn of work: split it via `todo_add_task` and complete the original. Do not silently absorb the extra work into the same task.

3.5. **Audit before verify.** Before moving on, call `todo_get_task_list`. If any task in the current phase is not `[✓]`, **STOP and audit.** You have drifted. Pick the next pending task in this phase and return to step 3. Do not run tests. Do not commit. The list must be clean before you proceed.

4.  **Verify.** At the end of the phase, run the full test suite. All tests must pass. Then re-read the execution plan — for every `[ ]` acceptance criterion, verify it's met and change to `[x]`. Fix any gaps before proceeding.

5.  **Commit and cleanup.**

    ```
    # e.g. for jinn (Fossil):
    fossil commit -m "<TASK> Phase N: <brief description>"   # the project's `commit` command
    fossil merge trunk                                       # the project's `sync-trunk` command (resolve conflicts, re-test, commit)
    ```

    Never merge your branch onto the project's main line.

6.  **Review.** _Append_ to the _execution plan_ file: **Changes** (what and why), **Divergence** (what didn't go to plan, or "None"), **Verification** (how verified), **Risks** (concerns or follow-up).

7.  **Annotate the spec.** Add strikethrough / divergence notes only. Never rewrite. For structural changes use `todo_add_phase`, `todo_postpone_task`, or `todo_cancel_task`.

8.  **Go to step 1.**

## When you are done

If the project defines `lint` and `format` commands, run them and fix every error and warning, then `commit` the result. For example, on jinn (Fossil + Rust):

```
just lint            # the project's `lint` command
just fmt-fix         # the project's `format` command
fossil commit -m "<TASK>: Lints fixed"   # the project's `commit` command
```

All reported errors and warnings must be fixed. If the project defines no `lint`/`format` commands, skip this step.
