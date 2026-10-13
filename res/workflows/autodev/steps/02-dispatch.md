# Dispatch — give claimable work to an idle worker.

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

## Dispatch

One `tell_session` per worker. The dispatch is self-contained — the
worker's reset behavior means it remembers nothing from run to run:

- the issue file's absolute path,
- the issue's id and its branch name (`autodev-issue-<id>` — the worker
  forks exactly this, per the crib sheet),
- the worker's checkout directory (absolute path),
- the integration branch name (`autodev-main`),
- the absolute path to this workflow's `steps/scm.md` (the worker has
  no other way to find it),
- the required report format (DONE/BLOCKED, per the worker's standing
  instructions — restate it; do not assume).

Mark the issue file the moment the dispatch is sent: `assignee: N`,
`status: dispatched`. You are the only writer of these fields.

## The tracker file

One file per issue, `.issues/<NNN>-<slug>.md` inside the orchestrator's
integration checkout, numbered sequentially by you. Frontmatter carries
the state:

    ---
    id: NNN
    status: triage | open | dispatched | done | parent | closed
    source: human | agent
    parent: NNN        # absent on top-level issues
    blocked-by: NNN, NNN   # absent when nothing blocks
    assignee: N        # absent when unassigned
    ---

Body: the goal, the approach when one is known, the verification to run.
Written for a stranger: the worker sees this file and its checkout,
nothing else.
