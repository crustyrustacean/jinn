# Integrate and verify — review finished work, then fold it in.

## When

Whenever a worker reports DONE. Two stages, strictly ordered: review
first, merge second. A branch that has not been APPROVEd is never
merged.

## The review gate

Dispatch the reviewer (per 02-dispatch.md's review payload) naming the
issue id, its contract path, and the branch. The issue stays
`dispatched` and keeps its `assignee` — the worker holds the issue
until the review says otherwise. End your turn; the reviewer's verdict
wakes you.

- **APPROVE** → proceed to the merge below.
- **REJECT** → re-dispatch the SAME worker on the SAME branch: the
  findings verbatim, the contract path, the instruction to sync from
  `autodev-main`, fix, re-verify, and re-report DONE. Do not edit the
  issue file beyond appending a review-round count if you keep one;
  findings live in the dispatch, the report, and the ledger. If the
  same issue takes four consecutive REJECTs, stop cycling it: set it
  back to `open`, clear the assignee, file the review findings as a
  new agent issue naming the non-convergence, and let the tracker
  re-triage it. Never resolve a finding by hand, never merge on a
  REJECT, never skip the review because a fix looks small.

## The merge

Work in your integration checkout (the one the integration lifecycle
setup gave you). Follow steps/scm.md's merge section exactly. The two
assertions that matter, every time:

- Your current branch is `autodev-main` before you merge (the crib
  sheet's check command). A commit from the wrong checkout lands on
  whatever branch that checkout holds — this is the one mistake that
  silently corrupts the run.
- Conflict check per the crib sheet's scoped recipe: read the merge
  output — it prints a WARNING line and prefixes conflicted files —
  never a tree-wide marker grep. On a real conflict: undo per the crib
  sheet, file a new issue naming the two sides that conflicted, and
  move on. Never resolve by hand, never commit markers.

## Verify

Two layers, both before the merge commit is final:

1. **The contract's verification section.** The contract lives in your
   own checkout (`.plans/<issue>/contract.md`): run its exact commands
   here, in the integration checkout, and check each one shows what
   the contract says it must show. These are the issue's acceptance
   criteria — defined by the planner, executed by the implementor,
   re-executed by you at the gate. A criterion that passes for the
   worker but fails here is a defect, whatever the worker's report
   claimed.
2. **The project's own gates.** The recipes you recorded in the ledger
   on first integration (`just check`, the suite). These catch what
   the contract's criteria don't reach.

Red at either layer: undo per the crib sheet if the merge is not yet
committed, otherwise file a new agent issue carrying the failing
output and set the just-merged issue back to `open` with a
`blocked-by` naming the new issue.

## After

Check the report's **contract divergences** section before closing
anything. A divergence is the implementor telling you the planner's
map of the code was wrong somewhere — file it as a note in the run
ledger, and if the divergence invalidates a sibling issue's contract
(they often share foundations), dispatch the planner to refresh that
contract BEFORE its issue goes out. Set the merged issue `status:
done` and clear its `assignee`. Its worker is idle again and may take
the next dispatch. When the last of an issue's children reaches
`done`, set the parent `status: closed`.

A worker that reports BLOCKED: set its issue back to `open`, clear the
assignee, and file the blocker as a new agent issue from the report's
content. The issue then competes for dispatch like any other — being
blocked once is not a demotion.

A worker silent for three consecutive scan windows: stop waiting on it.
Set its dispatched issue back to `open`, clear the assignee, and note
the abandonment in the run ledger. Do not cancel, message, or re-task
that worker this run; name it in the QA brief.
