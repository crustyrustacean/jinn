Dispatch the planner. The planner decomposes the goal into work items —
each an execution contract with a dependency graph.

## Dispatch

`tell_session` to the planner's session id with:

- The goal verbatim.
- The ledger path; contract files are written beside it as
  `<NN>-<slug>.md`, ordered by dependency.
- That each contract is standalone: the implementer sees it and the
  codebase, nothing else.

## Receiving the plan

The planner reports via `notify_parent`: contract paths, dependency
graph, risk notes. Then:

1. Read every contract. Fix nothing yourself.
2. Present the derived end state to the human — **G1**. One screen:
   the end state as checkable facts, the wave structure, the risks.
   Wait.
3. Approval → stamp each contract's origin as human-approved in the
   ledger, load the first wave's implementers, dispatch per
   `steps/02-implement.md`.
4. Changes → re-dispatch the planner with them verbatim. A changed
   contract re-presents at G1. No work item dispatches until approved.

## Done when

Every contract file exists beside the ledger, the ledger states the
dependency order, and the human has approved the plan.
