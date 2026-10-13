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
at or above it always forces decomposition.

When decomposing:

1. Write each sub-issue as a new tracker file: parent = this issue,
   `source: agent`, dependencies stated in `blocked-by`. Sub-issues
   cover the whole goal with nothing left unowned; anything the parent
   said that no sub-issue owns is a bug in your decomposition.
2. Set the parent's status to `parent` (it closes when all children
   close). Never delete the parent's body — children may quote it.
3. Order the children: foundation first, then the areas in dependency
   order. `blocked-by` carries that order.

When implementing as one issue, set status `open` (from `triage`).

## The triage status

A newly filed issue starts `status: triage`. Triage ends with exactly
one of: `open` (ready to dispatch), `parent` (decomposed, waiting on
children). A sub-issue you just filed also starts `triage` and is
triaged in this same pass — an issue never waits in `triage` between
orchestrator turns.

## Assignment test for the loop

After triaging, the issue is claimable when: status is `open`, and
every issue in its `blocked-by` list is closed. Dependency order is
data, never memory — check the files, not your recollection.
