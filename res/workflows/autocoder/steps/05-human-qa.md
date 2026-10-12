Present the behavioral summary for human QA — the G2 gate.

You are the orchestrator. All children have passed review. The human
tests behavior now; this is the run's second and last human gate. The
ledger carries everything; nothing else is needed.

## The summary

Write the QA summary into the ledger under a `## QA summary` heading,
and in your message to the human:

1. **What to test** — the observable behaviors, as concrete actions the
   human can perform, one line each, mapping each to the end-state
   lines it exercises.
2. **What changed** — branches merged into the run branch, one line
   each, in merge order. No diff reading required; behavior only.
3. **Risk notes realized** — any guess the plan flagged that turned out
   wrong, and what it changed.

## The gates

- The human replies **OK** → perform the run's single trunk write:
  merge the run branch into trunk from the root worktree, commit, and
  state that trunk advanced exactly once. Tear down nothing; leave the
  ledger and root worktree in place. The run is complete.
- The human reports a defect (toss-back) → log it in the ledger under
  `## QA toss-back`, write a fix-cycle contract against the **still
  alive** run branch, and re-enter the implementer. The run branch was
  never deleted; a fix cycle is stage 3 again with the new contract.
  Repeat this gate on every toss-back.

Never merge to trunk without the human's explicit OK, and never merge
anything but the run branch.

## Done when

Done when: trunk has advanced exactly once with the run branch merged,
or a toss-back is logged and dispatched as a fix cycle.
