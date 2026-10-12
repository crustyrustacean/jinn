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

## Done when

Done when: the ledger records the merge (or the stop and its
resolution), and the task list shows the work item's stage result.
