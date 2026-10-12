Dispatch the contract review for one agent-generated child — the gate
that stands in for the human the child never meets. This step is the
orchestrator's dispatch procedure; the reviewer's own behavior lives
in its seed pin.

## Composing the dispatch

Message the reviewer with tell_session, using the reviewer's session
id (recorded in your ledger at setup) with:

- The contract file path.
- The branch the child will be implemented on.
- That this is a plan review, before any code exists.

Say nothing else; the seed pin governs the rest.

## Receiving the verdict

The reviewer concludes its abbreviated verdict and reports back
through tell_session to your session id, binary:

- `contract approved` → proceed to step 3 with this contract.
- `contract returned`, with a numbered defect list → send the list
  to the planner with tell_session and re-run step 1 for that
  contract. Never implement against a returned contract.

Log the verdict in the ledger and reflect it into the task list.

## Done when

Done when: the ledger records either `contract approved` for the
contract as written, or `contract returned` with the defects routed
back to the planner.
