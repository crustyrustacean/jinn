# Land — the run's single write to the base branch.

## When

Only on the human's explicit OK to a QA brief. No OK, no landing —
this is the one step in the run that cannot proceed on its own.

## The sequence

Follow steps/scm.md's land section:

1. Open the land checkout per steps/scm.md: a one-shot subshell — no
   lifecycle setup is involved, your session's budget was spent at
   startup. Land dir: `<root>-land`, branch: the base branch.
2. In the land checkout (via a subshell or `cd` — you never move your
   session out of the integration checkout): merge `autodev-main`,
   check for conflict markers (grep — the merge exits zero even on
   conflict), commit.
3. Run the target project's check recipes once more, in the land
   checkout, before declaring the run landed. Red here is a defect:
   file it as an agent issue and resume the loop — the landing is
   simply not done yet.
4. Trash the land checkout with the guarded trash-move per steps/scm.md
   (it gates on repository identity and never moves your own seat).

## After

The run branch and the base branch now carry the same work. Write the
final run report in the conversation: the goal, the issue count, the
merge count, what landed, what was deferred (untriaged proposed issues
stay in the tracker — they are next cycle's seeds). Trash each worker
checkout per steps/scm.md's guarded trash-move, then close your
integration checkout per the crib sheet and name its directory in the
report for the human to delete at leisure.

Then idle. The run is over. The next run starts from the human's next
message, begins with `trash-purge`, and rebuilds everything — the
tracker is the only state that crosses runs, and it lives in the target
repo.
