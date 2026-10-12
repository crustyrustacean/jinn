Dispatch the contract review for one agent-generated child — the gate
that stands in for the human the child never meets. This step is the
orchestrator's dispatch procedure; the reviewer's own behavior lives
in its seed pin.

## Composing the dispatch

Message the reviewer (notify from you, or tell_session with the
reviewer's session id) with:

- The contract file path.
- The branch the child will be implemented on.
- That this is a plan review, before any code exists.

Say nothing else; the seed pin governs the rest.

## Receiving the verdict

The reviewer reports back through notify_parent with a binary
verdict:

- `contract approved` → proceed to step 3 with this contract.
- `contract returned`, with a numbered defect list → return the list
  to the planner via notify_parent and re-run step 1 for that
  contract. Never implement against a returned contract.

Log the verdict in the ledger and reflect it into the task list.

## Done when

Done when: the ledger records either `contract approved` for the
contract as written, or `contract returned` with the defects routed
back to the planner.
