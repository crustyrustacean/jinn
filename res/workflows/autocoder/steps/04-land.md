Land the run: the single base-branch write, on the human's OK.

## The merge

Work from your checkout (you hold the run branch):

1. Open the land checkout as a sibling (`<root>-land`):
   - **Fossil** — `run_lifecycle`, `script` = `worktree-open`, `args` =
     the dir and the base branch name.
   - **Git** — `run_lifecycle`, `script` = `worktree-land`, `args` =
     the dir and the base branch name. This creates a new branch
     `<base>-land` at the base tip, because git will not check out the
     base itself while another worktree holds it.
2. In the land checkout: `run_lifecycle`, `script` = `merge`, `args` =
   the run branch.
3. Verify there — the project's full check recipe — then commit the
   merge. (Git may fast-forward instead, leaving nothing to commit;
   that is a clean landing too.)
4. `run_lifecycle`, `script` = `advance`, `args` = the base branch
   name — from your checkout on git, from the land checkout on fossil.
   It fast-forwards the base branch and refuses a non-fast-forward.
5. Trash the land checkout per flow step 5. The base branch advanced
   exactly once; record the commit in the ledger.

## Refusals

A failed `merge` or `advance` stops the landing — resolve nothing
yourself. Record the failure and tell the human; the run branch is
intact and nothing has been lost.

## Done when

The ledger names the base-branch commit and the branch advanced exactly
once.
