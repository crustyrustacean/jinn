Implement one work item, review it, repeat until pass. This file covers
one item; run it for every item, concurrently within a wave.

## Dispatching an item

1. Load `attendants/implementer.toml` — one load per in-flight item.
2. `tell_session` to its session id with:
   - The contract path.
   - The sibling worktree dir: `<root>-<item-slug>`.
   - The branch name: the run branch's name plus `-<item-slug>`.
   - The run branch — what to fork from and sync with.
   - The absolute `lifecycle.toml` path and this session's id.

## Receiving the report

The implementer reports via `notify_parent`: **done** (branch name, what
changed) or **stopped** (what contradicts the contract). On stopped,
write a corrected contract or re-plan — never forward a guess about a
fix. Then load `attendants/reviewer.toml` and dispatch it with: the
branch name, the contract path, and the change summary.

## Review verdict

The reviewer reports via `notify_parent`, binary:

- **Pass** → merge the branch from your checkout, run the project's
  checks, commit, record the merge, retire both sessions, mark the item
  done, dispatch the next wave.
- **Fail** → re-dispatch the **same implementer** with the numbered
  defects, then review again.

## Circuit breaker

Three consecutive fails on one item: stop looping. Record the defects
in the ledger, park the item, and surface it to the human at QA with
the full history. Two agents in a loop agree with each other long
before they agree with the contract.

## Waves

Dispatch a wave only when every item before it has passed — later
contracts are written against earlier merges. Independent items in a
wave run concurrently; each gets its own implementer session, worktree,
and branch.

## Done when

Every work item in the wave has passed review, been merged onto the run
branch, and been recorded — or is parked at the circuit breaker for QA.
