# Triage and decompose — run whenever the tracker has an untriaged issue.

## Read the issue

Open the issue file. Establish three things:

- **Size.** Rough line count including tests if implemented as one unit.
- **Areas.** How many independent parts of the target repo it touches.
- **Blockers.** Issues that must close first, if any are already filed.

## Decompose or not

Decompose when size exceeds ~500 lines including tests, or the issue
touches three or more independent areas. Below that, implement as one
issue. Parallelism never justifies decomposing below the ceiling; size
at or above it always forces decomposition. When the goal allows both
shapes, bias toward independent children — a child nothing blocks on
keeps a worker busy that a chain would idle.

When decomposing:

1. Dispatch the planner on the issue FIRST (see 02-dispatch.md,
   contract-then-implement). Its contract pass sharpens the split:
   decompose along the boundaries its investigation found, not the
   ones a first read guesses at.
2. Write each sub-issue as a new tracker file: parent = this issue,
   dependencies stated in `blocked-by`. Sub-issues cover the whole
   goal with nothing left unowned; anything the parent said that no
   sub-issue owns is a bug in your decomposition.
3. Set the parent's status to `parent` (it closes when all children
   close). Never delete the parent's body — children may quote it.
4. Order the children: foundation first, then the areas in dependency
   order. `blocked-by` carries that order.

When implementing as one issue, set status `open` (from `triage`).

## The triage status

A newly filed issue starts `status: triage`. Triage ends with exactly
one of: `open` (ready to dispatch), `parent` (decomposed, waiting on
children). A sub-issue you just filed also starts `triage` and is
triaged in this same pass — an issue never waits in `triage` between
orchestrator turns.

## The tracker file

One file per issue, `.issues/<NNN>-<slug>.md` inside your own
checkout, numbered sequentially by you. **The id is always written
`NNN-<slug>` wherever it appears** — task list, reports, dispatches,
review verdicts, the ledger — never the bare number: the human reads
those surfaces too, and `003` alone tells them nothing. The slug is
two to five words, kebab-case, from the issue's title. Frontmatter
carries the state:

    ---
    id: NNN
    title: the issue in one line
    status: triage | open | dispatched | done | parent | closed
    parent: NNN            # absent on top-level issues
    blocked-by: NNN, NNN   # absent when nothing blocks
    assignee: <session-id> # absent when unassigned
    ---

Body: the goal, the approach when one is known, the verification to
run. Written for a stranger — but a stranger with a planner: the body
states WHAT and WHY, and the contract (planner's artifact) states the
how. Never write "the human" or "user said" into an issue; the
implementor neither knows nor cares where work came from. Where the
request came from is a fact for the run ledger, not the tracker. Keep
the frontmatter valid TOML: verify after every write — run 1 once
dropped `assignee` outside the block and needed a fix.

## Assignment test for the loop

After triaging, the issue is claimable when: status is `open`, and
every issue in its `blocked-by` list is closed. Dependency order is
data, never memory — check the files, not your recollection.
