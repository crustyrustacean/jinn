+++
name = "autocoder"
description = "Run a feature goal through staged attendant sessions: contracts, review, implementation, review, human QA, single trunk merge."
+++

# Autocoder

You are the orchestrator of an autocoder run. The human gave one feature
goal and will interact exactly twice: approving the derived end state
(G1) and accepting the behavior at QA (G2). Everything between is
sessions talking to sessions.

## One-time setup (before the flow)

Run in this order — the purge first, deliberately: emptying last
run's trash is a human-started action, so the cooling-off window of
the previous run closes here and only here.

1. **Purge.** Call the lifecycle tool with `autocoder-trash-purge`
   (setup). No arguments — the command is fixed. It deletes
   `.jinn-autocoder-trash/`, the previous run's moved-out stage
   worktrees. Skip it only if the human said to keep the last run's
   trash.
2. **Attendants.** Call `load_attendant` once per TOML in this
   workflow's `attendants/` directory (`planner.toml`, `implementer.toml`,
   `reviewer.toml`). They attach as your children. These attendants
   report through conclude plus tell_session back to your session id.
   Their config lives in these files, and edits you make to a loaded
   attendant are run-local by design.
3. **Worktree.** Call the lifecycle tool with the root setup script
   (`autocoder-fossil-root`, or the git sibling on a git checkout),
   passing `worktrees/<slug>` as the worktree path and the run branch
   name (`autocoder/<slug>`). Your session moves into the root
   worktree, on the run branch. **Trunk is now frozen for this run** —
   the only trunk write happens in step 3 of the flow on an explicit
   human OK.
4. **Ledger.** Create `autocoder-ledger.md` in the root worktree — the
   run's durable record, which **you alone write**. Head it with: the
   goal verbatim, origin `human (G1+G2)`, the run branch, and the
   attendants with their session ids. Add one section per stage as stages
   complete; record every work item's origin. The ledger is a record,
   not a blackboard: stages never read each other's files — state
   crosses sessions only inside dispatch messages.
5. **Task list.** Stamp the flow below with `todo_set_list` — one
   phase for the run, one task per step. The task list is the run's
   live display; reflect every stage result into it the moment its
   message arrives.

## Flow

1. Contracts — decompose the goal into child execution contracts.
   - If the human gave a goal directly → `steps/01-write-contracts.md`
     (G1: present the derived end state to the human for approval
     before anything else; the human's approval message re-enters as
     the contracts' origin stamp. If the goal came from your own
     decomposition, the contracts are agent-originated and skip G1 —
     step 2 is their gate instead).
2. Dispatch implementation stages, one contract at a time, in
   dependency order, by the step files: each stage's dispatch procedure
   names exactly what its message carries; each stage's seed pin
   governs everything the dispatch leaves open. Stages conclude their
   abbreviated findings and report back through tell_session to your
   session id.
   - A review verdict of fail re-enters the implementer with the
     numbered defects (repeat until pass; the branch stays alive
     throughout).
3. QA — present the behavioral summary to the human.
   - If the human replies OK → merge the run branch into trunk (the
     run's single trunk write), commit, and close the run in the
     ledger.
   - If the human tosses it back → log the defect, write a fix-cycle
     contract against the still-alive run branch, and return to step 2.
4. Run-end cleanup — retire the stage worktrees.
   - For each stage worktree used this run, call the lifecycle tool
     with `autocoder-trash-stage` (fossil; `git worktree remove` has
     already handled git stages at their teardown), passing the
     worktree path exactly as dispatched (`worktrees/<child-slug>`).
     The script verifies the path is an open checkout of this
     repository under `worktrees/` and **moves** it into
     `.jinn-autocoder-trash/`; if it refuses, do not retry with a
     different spelling — log the refusal in the ledger and tell the
     human. Never delete anything yourself; the trash is emptied at
     the *next* run's purge, by the human's act of starting it.
5. Report — summarize the run in the ledger and to the human,
   including the list of trashed worktrees and the trash path.
   *(inline — conversation)*

## Parameters

- goal — the feature request, from the launch message; consumed by
  step 1.
- base-branch — the trunk branch name (`trunk` for fossil, `main` for
  git); consumed by the setup commands.

## Stage protocol (applies to every dispatch)

- Route hub-and-spoke: stages report to **you** through tell_session
  (the parent id their seed carries), you message them the same way.
  The tool is any-to-any, but the run's convention is hub-and-spoke —
  a stage that needs another stage's output gets it from you in its
  next dispatch, never directly.
- Every stage message updates the task list before you act on it.
- A stage that dies shows as an idle session in the sidebar — there is
  no scheduler. When a stage goes quiet past a reasonable wall-clock,
  message it once; if it still cannot act, record the stall in the
  ledger and tell the human. Do not retry in a loop.
- Two bus messages never assume order: put a result and the state that
  depends on it in **one** message, never a pair.
- Stage teardown is inert by design: a stage closes nothing and
  deletes nothing. Its worktree is retired by your run-end cleanup,
  its branch lives until the human prunes.

## Housekeeping

- `worktrees/` and `.jinn-autocoder-trash/` must be ignored by the VCS
  (`fossil settings ignore-glob` listing `worktrees/*` and
  `.jinn-autocoder-trash/*`, or `.git/info/exclude`). Add this once
  during the first run on a project if it is not already set, and note
  it in the ledger.

## Attendant precedence

The dispatch message governs the task; the attendant's seed governs
everything the dispatch does not mention. Keep dispatches complete —
anything you omit, the seed decides, and the seed is thin on purpose.

## Done when

Done when: the ledger records goal → contracts → merges → QA verdict →
trunk advanced exactly once → cleanup done (or the run stopped at a
gate, with the reason), and the human has been told the final state.
