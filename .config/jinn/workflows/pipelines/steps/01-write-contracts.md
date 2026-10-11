Write the execution contract for a human-approved goal.

You are dispatched inside the pipeline run's root worktree, on the run
branch. The dispatch message carries the goal verbatim and the ledger
path.

## Derive, never ask

You are the G1 gate's drafting hand. The goal is human-originated: the
human has approved the end state and will not answer questions. Mine
the codebase for every decision the goal leaves open — conventions,
patterns, module placement — and fix them in the contract as decisions.
A decision you cannot verify from the code becomes a **risk note**
inside the contract, stated as a guess plus what to check first. Never
ask the human anything.

## The contract

One document per child work item, written to the ledger directory
alongside the ledger file (`NN-<slug>.md`, ordered by dependency). Each
contract is standalone — the implementer sees nothing else — and
contains, at minimum:

1. **End state** — checkable present-tense facts about observable
   behavior.
2. **Invariants** — what must hold during and after, with how each is
   checked.
3. **Constraints** — hard boundaries the implementer may not cross
   (untouched behavior, forbidden surfaces).
4. **Decision rules** — how to choose when the codebase offers options;
   the resolution order for mid-task problems.
5. **Verification** — the mechanical checks that prove the end state,
   named precisely enough to run.
6. **Size bound** — an estimate of the work. A child that cannot plausibly
   fit one focused session must be split into smaller children before
   the contract is final; say what it split into.
7. **Origin** — "human (approved by G1)" for the root, or "agent" for a
   child born of decomposition. Human-originated contracts note that
   G2 applies; agent-generated contracts run fully auto.

## Done when

Done when: every child contract file exists under the ledger
directory, the dependency order is stated in the ledger, and no
contract contains a question for the human.
