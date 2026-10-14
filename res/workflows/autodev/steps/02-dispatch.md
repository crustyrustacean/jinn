# Dispatch — give work to the pool, contract first.

## The two-phase dispatch

Every issue flows through the planner before any implementor sees it:

1. **Contract dispatch.** `tell_session` to the planner: the issue
   file's absolute path, the integration checkout path (where it saves
   the contract), the issue id. Mark the issue `status: contracting`
   while it runs.
2. **Implement dispatch.** When the planner reports READY, the issue
   is claimable again: mark it `open`, then dispatch an implementor
   per the payload below, naming the contract path.

One exception: triage of a decomposable root dispatches the planner
first too (01-triage.md) — the contract pass informs the split. Never
dispatch an implementor to an issue that has no contract. A missing
contract is a missing assignment, not a prompt to explore.

## Find claimable work

Scan the tracker. An issue is claimable when its status is `open` and
every issue named in its `blocked-by` is closed. Order claimable issues
by dependency (least blocked first), then by number (oldest first).

## Find an idle worker

A worker is idle when it is not named in any issue's `assignee` field.
At most one issue at a time carries a given worker's name. Never send a
second dispatch to a busy worker: one queued message is the most a
worker may hold, and you have no need of even that — the tracker holds
the queue. Unassigned claimable work simply waits.

## Implementor dispatch

One `tell_session` per worker. The dispatch is self-contained — the
worker's reset behavior means it remembers nothing from run to run:

- the issue file's absolute path,
- the contract file's absolute path (`.plans/<issue>/contract.md` in
  the integration checkout),
- the issue's id and its branch name (`autodev-issue-<id>` — the worker
  forks exactly this, per the crib sheet),
- the worker's checkout directory (absolute path),
- the integration branch name (`autodev-main`),
- the absolute path to this workflow's `steps/scm.md` (the worker has
  no other way to find it),
- the target repo's AGENTS.md or conventions file, if it has one,
- the required report format (DONE/BLOCKED, per the worker's standing
  instructions — restate it; do not assume).

Mark the issue file the moment the dispatch is sent: `assignee: N`,
`status: dispatched`. You are the only writer of these fields. Long
paths type badly under pressure — run 1 fat-fingered an attendant file
name once — so paste paths, never retype them.

## Planner dispatch payload

- the issue file's absolute path,
- the issue's id,
- the integration checkout's absolute path (the contract is saved
  under it, `.plans/<issue-id>-<slug>/contract.md`),
- the absolute paths of `steps/scm.md` and the target repo's
  conventions file,
- the reminder that its repo map persists (`preserve`) — refresh only
  what the dispatch says changed.

## Reviewer dispatch payload

Sent after every DONE report, before any merge (03-integrate.md's
review gate). The reviewer has no checkout — it reads the branch
through the repository file:

- the issue's id and its branch name,
- the contract file's absolute path,
- the absolute path of `steps/scm.md` (its review section tells it how
  to read a branch without a checkout),
- the repository file's absolute path (the orchestrator recorded it at
  startup),
- the target repo's conventions file, if it has one,
- the report's contract-divergences line, if any (the reviewer judges
  whether the divergence was honest).
