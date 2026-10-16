# SCM crib sheet

Reference for the repository commands used inside an autodev run. Read
the section you need; nothing here is a procedure to run top to bottom.

## Boundary of responsibility

**Branching, merge-back, and cleanup are the lifecycle script's job**
(`lifecycle.toml`, applied by the orchestrator via `run_lifecycle`). A
session never opens, merges, closes, or deletes a checkout itself:

| Event | Who runs what | What happens |
|---|---|---|
| Spawn | Orchestrator: `run_lifecycle` **setup** on the new session | Script branches `<parent>` (the orchestrator's branch), opens the implementor's checkout at `<dir>`, and the session is moved into it |
| Work | Implementor: plain commits in its own checkout (see "Committing") | Work lands on the implementor's branch only |
| Land | Orchestrator: `run_lifecycle` **teardown** on the implementor's session | Script merges `<parent>` in (conflict-gated), then merges the implementor's branch back onto `<parent>`, commits, closes the checkout, deletes the branch |

If a command you need is not in this file and is not one of those three
lifecycle operations, the gap is a workflow bug — report it, do not
improvise repository surgery.

This file and `lifecycle.toml` are the only places in this workflow that
name the SCM. Every other file stays SCM-blind; swapping SCMs later
means rewriting these two files and nothing else.

## Where am I

Run this first, any session, any time:

```sh
pwd
fossil branch current              # git: git branch --show-current
```

An implementor session should always be inside its own checkout
(`<dir>`, on `<your-branch>-issue-<id>`); the orchestrator is inside
its own seat, on the parent branch.

## Repository identity

```sh
fossil info | sed -n 's/^project-code: *//p'   # git: git remote get-url origin
```

The orchestrator records this as `repo-id` in the run ledger at
startup.

## The parent branch

Every implementor branches from the orchestrator's branch and merges
back onto it at teardown. The dispatch names the parent; an implementor
never guesses it. The parent is recorded in the run ledger at startup
and passed to the lifecycle setup as the `<parent>` argument.

## Committing (implementor, in its own checkout)

```sh
fossil addremove                   # git: git add -A
fossil commit -m "<what and why>"
fossil branch current              # confirm the commit landed on YOUR branch
```

The implementor's checkout holds only its own work, so a sweeping add
there is safe.

**Never sweep-add in the orchestrator's checkout.** That tree holds the
run state — the tracker (`.issues/`), plans (`.plans/`), and the
ledger (`.autodev.ledger.md`) — which the orchestrator ignored at
startup (fossil: `.fossil-settings/ignore-glob`; git:
`.git/info/exclude`). Ignored or not, never `addremove` / `git add -A`
in that tree: run state is working state, never committed.

## Clean status before teardown

Teardown's merge-back commits on the parent branch; it wants a clean
`fossil status` / `git status` on both sides. The implementor's own
commits cover its checkout. The orchestrator's checkout stays clean
because its run state is ignored, not untracked. If anything shows as
EXTRA/untracked at teardown time, it is either run state (ignore entry
missing — fix that, do not commit it) or real work someone abandoned
(file it as an issue).

## Reading a branch without a checkout (reviewer)

The reviewer has no checkout. It reads through the repository file
(fossil only; git needs no such step in a local-only repo):

```sh
fossil timeline <branch> -n 1 -R <repo-file>              # the branch tip
fossil diff --from <parent> --to <branch> --brief -R <repo-file>
fossil diff --from <parent> --to <branch> -R <repo-file>  # full patch
```

`-R` is read-only: it never opens, moves, or closes a checkout. (git:
`git diff <parent>..<branch>` from the orchestrator's checkout reads
local branches directly.) The full patch plus the execution plan is the
review's entire input: end state vs. delivered diff, tests vs. change,
scope vs. files-to-touch.

## Conflicts

`fossil merge` and `git merge` both **exit 0 on conflict** — the
failure is in the output and the files, never the exit code.

- The primary signal is the merge output itself: fossil prints `WARNING: N merge conflicts` and prefixes conflicted files with `MERGE`; git prints `CONFLICT` lines and leaves the index conflicted (`git diff --name-only --diff-filter=U` lists them).
- For confirmation, grep **only the files the merge named**. A naive tree-wide `grep -rl '<<<<<<<' .` matches build artifacts (`target/`, minutes of scanning) and this repo's own documented marker text.
- The lifecycle teardown script performs this check itself and exits `TEARDOWN-CONFLICT` before any irreversible step. When a manual merge (not teardown) hits a conflict: `fossil undo` restores the tree (git: `git merge --abort`). Never commit markers.

## Retiring a finished implementor's branch

Nothing to do. The lifecycle teardown closes and deletes the branch
after its merge-back succeeds — branch retirement is not a separate
step. A branch left open means its teardown never succeeded; the fix
is a successful teardown (see steps/03-integrate.md's conflict loop),
not branch surgery.

## Refreshing your own checkout (orchestrator, after every teardown)

A teardown merges the implementor's branch in the **repository** — your
session's checkout does not move by itself. Its working files are stale
the moment a teardown succeeds; update before you read, review, or QA
anything:

```sh
fossil update                       # git: git pull --ff-only
```

Fast, idempotent, and safe at any time on a clean tree. If it reports
nothing to do, nothing landed since your last update. Do this once per
successful teardown as part of the teardown step — the issue is not
`done` in your tree until your checkout shows the work. Never `fossil
update --force` / `git reset --hard` to "fix" a stale checkout: a dirty
tree means uncommitted run state or real work; resolve that first (see
"Clean status before teardown").
