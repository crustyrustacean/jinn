# Triage — run whenever the tracker has an untriaged issue.

## Read the issue

Open the issue file. Establish what is already known: the goal, any
blockers already filed, nothing more. Sizing and decomposition are the
planner's judgment, not yours — you have not read the code and must
not guess.

## Route it

Dispatch the planner on the issue (see 02-dispatch.md). One dispatch,
always the same instruction: the planner investigates and answers
either PLAN (atomic — the report carries the execution plan) or
DECOMPOSE (several units — the report carries the children). Mark the
issue `planning` while the planner holds it.

- **PLAN** → set status `open` (ready to dispatch).
- **DECOMPOSE** → transcribe the verdict; no judgment of your own:
  1. Write each child as a new tracker file from the verdict's blocks:
     title → slug, goal → body, boundary noted in the body,
     `blocked-by` from the verdict's order.
  2. Set this issue's status to `parent` (it closes when all children
     close). Never delete the parent's body — children may quote it.
  3. Each child starts `triage` and is routed in this same pass — an
     issue never waits in `triage` between orchestrator turns.
- **BLOCKED** → the planner could not plan or decompose; its report
  says what contradicts. Handle per the failure table (workflow.md
  Appendix F).

## The tracker file

One file per issue, `.issues/<NNN>-<slug>.md` inside your own
checkout, numbered sequentially by you. **The id is always written
`NNN-<slug>` wherever it appears** — task list, reports, dispatches,
review verdicts, the ledger — never the bare number: the human reads
those surfaces too, and `003` alone tells them nothing. The slug is
two to five words, kebab-case, from the issue's title. Frontmatter
carries the state:

    ---
    id = 1
    title = "the issue in one line"
    status = "triage"        # triage | open | planning | dispatched | done | parent | closed
    # parent = 1            # parent issue id; absent on top-level issues
    # blocked-by = [2, 3]   # issue ids that must close first; absent when nothing blocks
    # assignee = "<session-id>"  # absent when unassigned
    ---

Body: the goal, the boundary when the planner's verdict drew one, the
verification to run. Written for a stranger — but a stranger with a
planner: the body states WHAT and WHY, and the execution plan (the
planner's artifact) states the how. Never write "the human" or "user
said" into an issue; the implementor neither knows nor cares where
work came from. Where the request came from is a fact for the run
ledger, not the tracker. Keep the frontmatter valid TOML: verify after
every write.

## Assignment test for the loop

After triaging, the issue is claimable when: status is `open`, and
every issue in its `blocked-by` list is closed. Dependency order is
data, never memory — check the files, not your recollection.
