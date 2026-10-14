# SCM crib sheet — the in-turn command surface.
#
# This file and lifecycle.toml are the ONLY places in this workflow that
# name the SCM. Every other file stays SCM-blind; swapping the SCM later
# means rewriting these two files and nothing else.
#
# Everything here runs as plain shell inside a session's own checkout.
# The two checkout openings that need the harness (`run_lifecycle`,
# which moves a session into its new checkout) are in lifecycle.toml;
# everything else — probing, branching, merging, trashing, the land
# checkout — is plain commands here.

## Where am I (run this first, any session, any time)

    pwd
    fossil branch current

## Repository identity

    fossil info | sed -n 's/^project-code: *//p'

The orchestrator records this as `repo-id` at startup; the trash gate
compares it.

## Fork the integration branch (orchestrator, at startup, from the
## starting checkout — BEFORE its integration-open lifecycle setup)

    fossil branch new autodev-main <starting-branch>

## Fork a worker's standing branch (orchestrator, at startup, from the
## starting checkout — BEFORE that worker's worker-open lifecycle setup)

    fossil branch new autodev-worker-N autodev-main

## Open the land checkout (orchestrator, landing only — one-shot subshell;
## no session has lifecycle budget left by then, and none needs it: the
## orchestrator reads and writes through an absolute path and cd's in a
## subshell, never moving its own session)

    ( mkdir -p <root>-land && cd <root>-land \
      && fossil open "$(fossil info | sed -n 's/^repository: *//p')" <base-branch> )

## Fork a fresh work branch for a new issue (worker, in own checkout)

    fossil branch new <issue-branch> autodev-main
    fossil update <issue-branch>

Commit from then on lands on `<issue-branch>`. A branch name that
already exists fails with a clear refusal — that is the signal to
`fossil update` onto it (reattaching to leftover work), not to invent a
variant name.

## Sync the integration branch into your checkout (worker, start of an issue)

    fossil merge autodev-main
    # resolve any conflicts in the working tree, then:
    fossil commit -m "sync autodev-main"

If the merge wrote conflict markers you cannot resolve, run
`fossil undo` to restore the tree and report the conflict instead of
committing one.

## Commit (worker, when the issue's work is done)

    fossil addremove
    fossil commit -m "<what and why>"
    fossil branch current   # confirm the commit landed on YOUR branch

The worker's checkout holds only its own work, so a sweeping add there
is safe. The ORCHESTRATOR's checkout holds the tracker (`.issues/`) and
the ledger as untracked files — plain merge+commit ignores untracked
files (verified), so never run `addremove` or a bare `fossil add .` in
that tree; the integrate/land commits need no add at all, because a
merge stages its own changes:

    fossil commit -m "integrate <issue-branch>"   # merge already staged the changes

## Merge a finished worker branch (orchestrator, in its integration checkout)

    fossil branch current        # MUST print autodev-main — commits land on
                                 # whatever branch this checkout holds
    fossil merge <issue-branch>
    # conflict check HERE, before the commit — see the scoped recipe below
    fossil commit -m "integrate <issue-branch>"

A repeated merge of an already-integrated branch is a no-op refusal —
safe to retry, never an error.

## Conflict-marker check (scoped — read this before any grep)

A conflicted merge exits zero, so you must check between merge and
commit — but a NAIVE `grep -rl '<<<<<<<' .` is worse than no check:

- `target/` build artifacts match and take minutes to scan (run 1 lost
  six minutes to this once).
- This repo's own `justfile` and this workflow's `scm.md` contain the
  literal `<<<<<<<` as documented text — they match forever.

The primary signal is the merge output itself: `fossil merge` names
every file it touched (`UPDATE` / `MERGE`) and prints
`WARNING: N merge conflicts` explicitly when a conflict exists. So the
procedure is:

1. Read the merge output. Any `MERGE` line or `WARNING` is a conflict:
   `fossil undo`, file a new issue naming the two sides, move on.
2. For extra confirmation only, grep the files the merge named — never
   the whole tree:

       fossil merge <issue-branch> | tee /tmp/autodev-merge.log
       grep -oP '^(?:MERGE|EDITED|ADDED|UPDATE)\s+\K.*' /tmp/autodev-merge.log \
         | xargs -r grep -l '<<<<<<<'

   (Verified against a live conflict: the WARNING line and the `MERGE`
   prefix are what fossil actually prints.) Expect legitimate hits in
   `justfile` and any file whose content documents markers. Treat the
   grep as confirmation; the merge output is the authority. On a real
   conflict: `fossil undo`, file a new issue naming the two sides,
   move on.

## Review a branch without a checkout (reviewer; read-only, -R on the
## repository file — never opens a checkout, never writes anything)

    fossil timeline <branch> -n 1 -R <repo-file>     # the branch tip
    fossil diff --from autodev-main --to <branch> --brief -R <repo-file>
    fossil diff --from autodev-main --to <branch> -R <repo-file>   # full patch

The full patch plus the contract is the review's entire input: end
state vs. delivered diff, tests vs. change, scope vs. files-to-touch.
Everything here is read-only; `-R` never opens, moves, or closes a
checkout.

## Land the run (orchestrator, in the land checkout via a subshell,
## only on human OK)

    cd <root>-land
    fossil branch current        # MUST print the base branch
    fossil merge autodev-main | tee /tmp/autodev-merge.log
    # conflict check exactly as the integrate recipe above: WARNING or
    # MERGE lines in the log → fossil undo, abort the landing
    fossil commit -m "autodev run: <goal summary>"

## Trash a finished run checkout (orchestrator, after landing; guarded)

Trash-move ONLY worker checkouts and the land checkout — never your own
integration checkout (you sit in it; a moved working directory breaks
every later command). Gates, all required:

    ADIR="<checkout-path>"; ADIR="${ADIR%/}"
    ROOT="$(cd "$ADIR" && fossil info 2>/dev/null | sed -n 's/^local-root: *//p')"
    [ -n "$ROOT" ] || { echo "not a checkout: $ADIR"; exit 1; }
    ROOT="${ROOT%/}"
    WID="$(cd "$ADIR" && fossil info 2>/dev/null | sed -n 's/^project-code: *//p')"
    [ "$WID" = "<repo-id>" ] || { echo "wrong repository: $ADIR"; exit 1; }
    SELF="$(fossil info | sed -n 's/^local-root: *//p')"; SELF="${SELF%/}"
    [ "$ADIR" = "$SELF" ] && { echo "refusing: that is this checkout"; exit 1; }
    case "$ADIR/" in "$SELF"/*) { echo "refusing: that is inside this checkout"; exit 1; } ;; esac
    PARENT="$(dirname "$ADIR")"
    mkdir -p "$PARENT/.jinn-autodev-trash" && mv "$ADIR" "$PARENT/.jinn-autodev-trash/"

The move (not delete) is the point: the worst case that passes every
gate is recoverable by hand from `.jinn-autodev-trash/`, and the next
run's startup purge is the only thing that empties it.

## Close your own integration checkout (orchestrator, after landing)

    fossil close --force

The directory remains as plain files; name it in the run report so the
human can delete it at leisure. Never trash-move the tree you sit in.
`fossil close` acts on the tree you are IN — it takes no path argument —
so the subshell form above is the only safe way to touch another
checkout.

## Purge the trash (orchestrator, startup only — a human-started run
## closes the cooling-off window)

The trash always sits one level above the checkout you start in, under
its fixed name — a literal path, no variables:

    ls -d ../.jinn-autodev-trash    # may not exist on a first run
    ls ../.jinn-autodev-trash       # every entry should be a trashed
                                    # run checkout (*-worker-N, *-land)
    rm -rf ../.jinn-autodev-trash

Look at the second listing before deleting. If anything inside does not
look like a trashed run checkout — the names a run creates are
`<root>-worker-N` and `<root>-land` — stop and tell the human instead
of deleting. Never purge any other path, and never purge after startup:
the purge runs once, before any checkout of this run exists.
