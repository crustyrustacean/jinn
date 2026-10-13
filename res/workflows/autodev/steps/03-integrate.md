# Integrate and verify — fold finished worker branches into the run branch.

## When

Whenever a worker reports DONE, and before any dispatch that would
build on that work.

## The merge

Work in your integration checkout (the one the integration lifecycle
setup gave you). Follow steps/scm.md's merge section exactly. The two
assertions that matter, every time:

- Your current branch is `autodev-main` before you merge (the crib
  sheet's check command). A commit from the wrong checkout lands on
  whatever branch that checkout holds — this is the one mistake that
  silently corrupts the run.
- After the merge, check for conflict markers. A conflicted merge
  exits zero anyway, so the marker grep is the only real signal. On
  markers: undo per the crib sheet, file a new agent issue naming the
  two sides that conflicted, and move on. Never resolve by hand, never
  commit markers.

## Verify

Run the target project's own check recipes from the integration
checkout. Which recipes: read the target repo the first time you
integrate — its build files, its task runner, its docs — and record
your choice in the run ledger. All green before the merge commit is
final. Red: undo per the crib sheet if the merge is not yet committed,
otherwise file a new agent issue carrying the failing output and set
the just-merged issue back to `open` with a `blocked-by` naming the
new issue.

## After

Set the merged issue `status: done` and clear its `assignee`. Its
worker is idle again and may take the next dispatch. When the last of
an issue's children reaches `done`, set the parent `status: closed`.

A worker that reports BLOCKED: set its issue back to `open`, clear the
assignee, and file the blocker as a new agent issue from the report's
content. The issue then competes for dispatch like any other — being
blocked once is not a demotion.

A worker silent for three consecutive scan windows: stop waiting on it.
Set its dispatched issue back to `open`, clear the assignee, and note
the abandonment in the run ledger. Do not cancel, message, or re-task
that worker this run; name it in the QA brief.
