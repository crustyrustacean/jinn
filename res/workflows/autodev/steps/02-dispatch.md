# Dispatch — spawn an implementor for contract-ready work.

## Contract first, always

Every issue flows through the planner before any implementor session
exists for it:

1. **Contract dispatch.** `tell_session` to the planner: the issue
   file's absolute path, your checkout path (where it saves the
   contract), the issue id. Mark the issue `status: contracting` while
   it runs.
2. **Spawn.** When the planner reports READY, the issue is claimable:
   mark it `open`, then spawn an implementor per the procedure below.
   Never spawn for an issue without a READY contract — a missing
   contract is a missing assignment, not a prompt to explore.

One exception: triage of a decomposable root dispatches the planner
first too (01-triage.md) — the contract pass informs the split.

## Find claimable work

Scan the tracker. An issue is claimable when its status is `open` and
every issue named in its `blocked-by` is closed. Order claimable issues
by dependency (least blocked first), then by number (oldest first).
Concurrent implementors are fine when the issues are file-disjoint —
triage's contracts fence territories (each contract names the files it
owns), so disjointness is already on disk. When two claimable issues
touch the same files, serialize them: the second waits for the first's
teardown, because parallel teardowns onto the same parent would fight.

## Spawning an implementor

One session per issue. The procedure, in order:

1. Create the session (a plain new session in your run).
2. `run_lifecycle` setup `autodev implementor (fossil)` — or the git
   variant if the target repo is git — with:
   - `path` = this workflow's `lifecycle.toml` (absolute),
   - `session_id` = the new session's id,
   - `dir` = a fresh checkout directory beside your own, named for the
     issue: `<your-root>-issue-<id>`,
   - `branch` = the issue's branch: `<your-branch>-issue-<id>`
     (derived from your branch — never a fixed prefix. Branches named
     after the workflow collide across runs and with a run's own
     leftovers; a branch named after its parent cannot collide),
   - `parent` = your branch (the one recorded in the ledger at
     startup).
   The runner moves the session into its checkout.
3. Send the dispatch via `tell_session`.

Paste every path; never retype one — long paths type badly under
pressure and a fat-fingered path is a spawn failure (run 1
fat-fingered an attendant file name once).

## Implementor dispatch payload

The implementor's reset is gone — it is preserve now — but keep the
dispatch self-contained anyway: a dispatch must be actionable on its
own, even though the session remembers the previous one. For a fresh
spawn:

- the issue file's absolute path,
- the contract file's absolute path (`.plans/<issue>/contract.md` in
  your checkout),
- the issue's id and its branch name,
- the implementor's checkout directory (absolute path — the lifecycle
  setup printed it; it is also the session's working directory),
- your branch name (the parent its teardown will merge into),
- the absolute path to this workflow's `steps/scm.md`,
- the target repo's AGENTS.md or conventions file, if it has one,
- the required report format (DONE/BLOCKED — restate it; do not
  assume).

For a re-dispatch to the same session (REJECT findings, teardown
conflict fixes): shorter, but still explicit — what failed and why,
the findings or conflict files verbatim, what to do, and the same
report format.

Mark the issue file the moment the spawn is live: `assignee` = the
session id, `status: dispatched`. You are the only writer of these
fields.

## Planner dispatch payload

- the issue file's absolute path,
- the issue's id,
- your checkout's absolute path (the contract is saved under it,
  `.plans/<issue-id>-<slug>/contract.md`),
- the absolute paths of `steps/scm.md` and the target repo's
  conventions file,
- the reminder that its repo map persists (`preserve`) — refresh only
  what the dispatch says changed.

## Reviewer dispatch payload

Sent after every DONE report, before any teardown (03-integrate.md's
review gate). The reviewer has no checkout — it reads the branch
through the repository file:

- the issue's id and its branch name,
- the contract file's absolute path,
- the absolute path of `steps/scm.md` (its review section tells it how
  to read a branch without a checkout),
- the repository file's absolute path (recorded in your ledger at
  startup),
- the target repo's conventions file, if it has one,
- the report's contract-divergences line, if any (the reviewer judges
  whether the divergence was honest).
