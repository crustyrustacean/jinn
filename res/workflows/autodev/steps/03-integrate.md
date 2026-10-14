# Review, teardown, and the retry loops.

## When

Whenever an implementor reports DONE. Strictly ordered: review first,
teardown second, and only an APPROVE ever reaches teardown.

## The review gate

Dispatch the reviewer (per 02-dispatch.md's review payload) naming the
issue id, its contract path, and the branch. The issue stays
`dispatched` and keeps its `assignee`. End your turn; the reviewer's
verdict wakes you.

**Your observations ride the payload; the verdict stays the
reviewer's.** If you noticed anything the report alone would not show
— a gate anomaly, a ledger contradiction, baseline-vs-baseline
accounting you could not verify — include it in the reviewer dispatch,
attributed "orchestrator observation", verbatim and without judgment.
Never pre-empt the verdict yourself: no self-REJECT, no self-APPROVE,
no teardown before the reviewer speaks.

- **APPROVE** → proceed to teardown below.
- **REJECT** → re-dispatch the SAME implementor session (preserve keeps
  it warm on its own code): the findings verbatim, the contract path,
  the instruction to fix, re-verify per the contract, and re-report
  DONE. Do not edit the issue file beyond a review-round count if you
  keep one. If the same issue takes four consecutive REJECTs, stop
  cycling it: the implementor's work sits on its own branch, harming
  nothing — set the issue back to `open`, file the findings as a new
  issue, and let the tracker re-triage. The branch and its checkout
  wait for whichever issue eventually consumes them; teardown only
  ever follows an APPROVE. Never resolve a finding by hand, never
  teardown on a REJECT, never skip the review because a fix looks
  small.

## The teardown

One `run_lifecycle` teardown call on the implementor's session — no
arguments; the session's recorded coordinates (`dir`, `branch`,
`parent`) drive it. The script merges the parent in, gates on
conflicts, then merges the work back onto your branch, commits, closes
the checkout, and removes it. Then refresh your own checkout (`fossil
update` / `git pull --ff-only` — steps/scm.md's refresh section); the
teardown moved the branch, not your working files. The issue is `done`
the moment this
succeeds: the work is on your branch, the session is gone, the branch
is closed.

### Teardown conflict — the implementor fixes it

A teardown that exits with `TEARDOWN-CONFLICT` changed nothing
irreversible: the implementor's session and checkout are exactly as
they were. The fix loop:

1. Re-dispatch the SAME implementor: which files conflicted (from the
   script's output), that these are merge conflicts against your
   branch's current content, and the instruction to resolve, verify
   per the contract, commit, and report DONE.
2. On its report, re-issue teardown. The re-run's merge is incremental
   — previously-merged content is a no-op, only the newly fixed files
   are at stake.

Do not fix conflicts yourself, do not file them as tracker issues, and
do not abandon the session over one: the implementor knows the code it
wrote, which is why it is preserve.

### Teardown success — refresh your own seat

After every successful teardown: `fossil update` (git:
`git pull --ff-only`) in your own checkout. The merge happened in the
repository; your working tree stays stale until you update, and every
later action (reviewing another diff, writing the QA brief, answering
questions about current code) reads that tree. Skip the refresh and
you will review and QA against yesterday's code.

### Teardown failure that is NOT a conflict

A teardown failing for another reason (bad coordinates, a dirty tree
the auto-commit could not settle) is infrastructure, not work: read
the error, correct the invocation, and re-issue. If a checkout is
genuinely stuck, keep the issue `dispatched`, note the session id in
the ledger, and continue other work; tell the human in the next QA
brief's Known state. Never hand-work around a broken teardown — the
branch's content is not on your branch until the script says so.

## After

Set the merged issue `status: done` and clear its `assignee`. When the
last of an issue's children reaches `done`, set the parent
`status: closed`.

A worker that reports BLOCKED: its session holds a checkout and a
branch with partial work. Do not teardown — the work is not approved
and not wanted as-is. Set the issue back to `open`, file the blocker
as a new agent issue from the report's content, and note the stuck
session in the ledger under Known state (named in the QA brief). A
blocked session is abandoned, never messaged again this run; its
checkout is debris the human can remove, and its branch is closed at
your leisure — never by you mid-run.

A worker silent long past its expected effort (hours with no report on
a contract-sized issue): treat as abandoned — same handling as BLOCKED,
with the silence noted in the QA brief. There is no remote cancel; do
not message it again.
