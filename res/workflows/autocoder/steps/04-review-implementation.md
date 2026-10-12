Dispatch the review of one implemented contract. This step is the
orchestrator's dispatch procedure; the reviewer's own behavior —
criterion-by-criterion verification with evidence, binary verdicts —
lives in its seed pin.

## Composing the dispatch

Message the reviewer (notify from you, or tell_session with the
reviewer's session id) with:

- The implementer's branch name.
- The contract file path.
- The implementer's change summary (a fix cycle adds what changed
  since the last review).

Say nothing else; the seed pin governs the rest.

## Receiving the verdict

The reviewer reports back through notify_parent with a binary
verdict:

- **pass** — the branch is merged-ready. Log the pass in the ledger
  and move to step 5 when every work item of the run has passed.
- **fail** — a numbered list of unmet criteria with evidence gaps.
  Re-dispatch the implementer (step 3) with that list verbatim as the
  fix-cycle assignment, then review again. Repeat until pass; the
  branch stays alive throughout.

Log every verdict in the ledger and reflect it into the task list.

## Done when

Done when: the ledger records the verdict, and the work item is either
merged-ready or back in step 3 with the numbered defects.
