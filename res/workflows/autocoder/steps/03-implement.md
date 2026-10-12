Implement one execution contract inside your own worktree.

You are dispatched as the implementer. The dispatch message carries the
contract file path, your worktree path, your branch name, and the base
branch to fork from. The full contract is your assignment — read it
first, end to end.

## Setup

1. Run the lifecycle tool (`run_lifecycle`) with `path` set to the
   workflow's `lifecycle.toml`, `script` set to the stage setup script
   (`autocoder-fossil-stage`), passing your worktree path, branch
   name, and the base branch from the dispatch. Your session moves
   into the worktree. Work nowhere else.
2. Read the contract. If any part of it is wrong about the codebase,
   stop and message the orchestrator with the contradiction — do not
   improvise past a broken contract.

## Build

Follow the contract's constraints, decision rules, and resolution
order. Build through the project's `just` recipes; iterate with
`test-one`; never commit red. Tests assert observable behavior.

## Finish

1. Confirm every end-state line and every verification check in the
   contract passes.
2. Commit on your branch.
3. From inside your worktree, merge your branch into the base branch
   the dispatch named. **Never trunk** — trunk is written once per run,
   by the orchestrator at QA acceptance.
4. Run the lifecycle tool with `script` set to the stage teardown
   script (same `lifecycle.toml` path as setup) to close your
   checkout and remove the worktree.
5. Message the reviewer: your branch name, the contract path, and a
   summary of what changed. A fix cycle adds what changed since the
   last review.

## Done when

Done when: the reviewer has received your branch name and summary, your
worktree is torn down, and your branch's changes are merged into the
base branch — and trunk is exactly as it was.
