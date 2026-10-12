Dispatch the planner: decompose the human-approved goal into child
execution contracts. This step is the orchestrator's dispatch
procedure; the planner's own behavior lives in its seed pin.

You run this inside the run's root worktree, on the run branch.

## Composing the dispatch

Message the planner with tell_session, using the planner's session
id (recorded in your ledger at setup) with:

- The goal verbatim — the human's words, unedited.
- The ledger path, and that contract files are written to the ledger
  directory as `NN-<slug>.md`, ordered by dependency.
- The instruction that each contract is standalone — the implementer
  will see nothing else.

Say nothing else. The planner's seed pin governs everything the
dispatch leaves open; a thin dispatch plus the thin seed is the design.

## G1 — if the goal is human-originated

Before dispatching, present the human with the derived end state and
wait for approval. The human's approval message is the contracts'
origin stamp: human-originated contracts run through G2 (step 5) —
note that on each. If the goal came from your own decomposition, the
contracts are agent-originated: skip G1 and let step 2 review them
instead.

## Receiving the result

The planner concludes its abbreviated findings and reports back
through tell_session to your session id: contract file paths,
dependency order, and risk notes. When it arrives:

1. Read each contract; confirm none contains a question for the human.
2. Record the dependency order and every work item's origin in the
   ledger.
3. Reflect completion into the task list.

## Done when

Done when: every child contract file exists under the ledger
directory, the ledger states the dependency order, and the task list
shows step 1 complete.
