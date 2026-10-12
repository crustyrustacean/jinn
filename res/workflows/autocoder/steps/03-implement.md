Dispatch the implementation of one contract. This step is the
orchestrator's dispatch procedure; the implementer's own behavior —
worktree setup, merge rules, never committing red — lives in its seed
pin.

## Composing the dispatch

Message the implementer with tell_session, using the implementer's
session id (recorded in your ledger at setup), with everything it
needs, since it invents nothing:

- The contract file path.
- Its worktree path: `worktrees/<child-slug>`.
- Its branch name.
- The base branch to fork from and merge into: the run branch. (The
  seed forbids trunk on its own; naming the run branch here is what
  makes that hold.)

Say nothing else; the seed pin governs the rest.

## Receiving the report

The implementer concludes its abbreviated findings and reports back
through tell_session to your session id:

- **Merged** — branch name, contract path, change summary. Dispatch
  step 4 with the same three facts.
- **Stopped** — what contradicts the contract. Do not forward your
  own guess about a fix: write a corrected contract (step 1) or
  return the numbered defects (step 4), by what the contradiction
  calls for. Log the stop in the ledger.

A fix cycle is this step again, same worktree path and branch, with
the fix-cycle contract — the branch stays alive throughout.

Implement one execution contract inside your own worktree.

You are dispatched as the implementer. The dispatch message carries the
contract file path, your worktree path, your branch name, and the base
branch to fork from. The full contract is your assignment — read it
first, end to end.

## Setup

1. Run the lifecycle tool with the stage setup script, passing your
   worktree path, branch name, and the base branch from the dispatch.
   Your session moves into the worktree. Work nowhere else.
2. Read the contract. If any part of it is wrong about the codebase,
   stop and message the orchestrator with the contradiction — do not
   improvise past a broken contract.

## Build

Follow the contract's constraints, decision rules, and resolution
order. Build through the project's `just` recipes; iterate with
`test-one`; never commit red. Tests assert observable behavior.

## Finish

1. Confirm every end-state line and every verification check in the
   contract passes.
2. Commit on your branch.
3. From inside your worktree, merge your branch into the base branch
   the dispatch named. **Never trunk** — trunk is written once per run,
   by the orchestrator at QA acceptance.
4. Run the lifecycle tool with the stage teardown script to close your
   checkout and remove the worktree.
5. Message the reviewer: your branch name, the contract path, and a
   summary of what changed. A fix cycle adds what changed since the
   last review.

## Done when

Done when: the ledger records the merge (or the stop and its
resolution), and the task list shows the work item's stage result.
