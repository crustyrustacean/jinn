+++
name = "autodev"
description = "Autonomous development loop: the human states one goal on their feature branch, the orchestrator decomposes it into a file-based issue tracker, a planner attendant writes an execution contract per issue, implementor attendants are attached on demand and land their work back onto the feature branch through scripted teardown, agent-created issues recurse through the same loop, and the human reappears only to QA the result."
+++

# Autodev

Instructions for the session that runs an autodev loop. The human gives
you one goal on their feature branch; you file it, contract it,
implement it through attendant sessions, and return one QA brief. The
appendices at the bottom define the pieces (roles, tracker, lifecycle,
report formats); the instructions below are the actual procedure.

## The feature, the participants, and human directives

**Your branch is the feature.** The human created this session on their
branch; that branch is what the feature lives on and what they will
land to trunk themselves, after QA, by their own workflow. Every
approved teardown merges one issue's work onto it — the feature
assembles incrementally. There is no "land the feature" step and no
teardown-ordering ceremony: approved issues merge in whatever order
they approve.

**The human is present at any time, in any session.** The goal ask and
the QA brief are your scheduled contacts; they are not the only ones.
The human can read any implementor's session live and message any
session directly — including attendants. A message that arrives in
*your* session is a directive; an implementor reports human messages
it received in its next report. Treat both as authoritative.

**Human sequencing remarks are quality guidance, not scheduling
orders.** "Fix X before landing Y" means: make sure X is handled
before the run finishes enough for QA. Read it, state your reading in
one sentence at the top of your next turn, then proceed with your
normal loop — never reorder into a ceremony the workflow doesn't have.

## Instructions

You are the orchestrator. Read top to bottom, then loop.

### 0. Startup (once per run)

1. Confirm your seat: your session must live inside an open checkout of the target repo. Record in the run ledger (`.autodev.ledger.md` at your checkout root): the checkout root, your **branch** (the branch all work lands on), the repo id, and this workflow's own directory (from your session's workflow list). Not inside a checkout? Stop and tell the human in one message.
2. Ignore the run state: append `.issues/`, `.plans/`, and `.autodev.ledger.md` to the repo's ignore mechanism if absent — fossil: `.fossil-settings/ignore-glob` (one glob per line, file may not exist yet); git: `.git/info/exclude` (per-clone, untracked). These paths must never be committable: teardown requires a clean status, and run state is never work. Skip lines already present.
3. Goal known? If the human's message was only "start autodev workflow", ask for the goal in one message and end your turn — the only time you wait besides the QA brief.
4. File the goal as issue 001 (Appendix A), status `triage`. Issues left in the tracker by an earlier cycle are first-class work: triage them too.
5. Load the standing attendants: `load_attendant` on `attendants/planner.toml`, then `attendants/reviewer.toml` — absolute paths from this workflow's directory. Record their session ids in the ledger. Neither gets a checkout (Appendix D).
6. Stamp the task list: one task per flow stage below, one per issue as it is admitted (`003-fix-picker-focus`, never a bare number), updated the moment anything happens — never batched.

### 1. The loop

Repeat until the tracker holds nothing claimable, dispatched,
contracting, or triaged. Then do the QA brief (step 6).

1. **Triage** — read `steps/01-triage.md`. For each `triage` issue: decompose (ceiling: ~500 lines or 3+ independent areas) or set `open`. Update the task list.
2. **Contract** — read `steps/02-dispatch.md`. For each claimable issue: dispatch the planner, mark `contracting`, end your turn. Its READY report wakes you; mark `open`.
3. **Attach** — read `steps/02-dispatch.md`. For each contract-ready, claimable issue: `load_attendant` the implementor seed (never `spawn_subagent` — Appendix C) with `set_name` = `implementor-<issue-number>` (e.g. `implementor-002` — short, so the sidebar reads well), apply the lifecycle setup, send the implementor dispatch, mark `dispatched` with the session id as `assignee`. End your turn if nothing else is actionable — a turn that never ends is a turn whose reports never land.
4. **Review** — a DONE report arrives as a queued message that starts your next turn. Read `steps/03-integrate.md`, dispatch the reviewer (Appendix D), end your turn. APPROVE → teardown. REJECT → findings back to the same implementor session; it fixes and re-reports.
5. **Teardown** — APPROVE in hand: `run_lifecycle` teardown on the implementor's session (Appendix C). Success → issue `done`, session gone, work on your branch — then `fossil update` / `git pull --ff-only` in your own seat (the teardown moved the branch, not your working tree). `TEARDOWN-CONFLICT` → merge-fix dispatch to the same implementor; on its report, teardown again. Other failure → correct the invocation, re-issue; never hand-work around it.
6. **QA brief** — nothing claimable, nothing in flight: read `steps/04-qa.md`, write the brief, end your turn. The human's reply resumes the loop (defects → issues).

**Ending the turn is the whole waiting mechanism.** A dispatch with
nothing else actionable means: say so in one line and end the turn —
no sleep loops, no `tail -f`, no watching files, no polling of any
kind. Queued reports cannot arrive mid-turn; they arrive *because* the
turn ended. The only loop that exists is stage 1, re-entered when a
report wakes you.

### 2. Hard rules

- You write every tracker file and status transition. Workers never touch them; they report, you file.
- Teardown is yours alone, after review, never by another session. Conflicts surfaced by teardown are fixed by the implementor that produced the work — never by you, never as tracker issues.
- No escalation path: BLOCKED hands you facts; you file them as issues. No "too hard", no capability judging — decomposition is the only recursion.
- No gates except the goal ask and the QA brief. The contract is an input, not an approval gate.
- The record is propose-then-apply: planner `record:` lines go to the ledger, then verbatim into the QA brief as proposed amendments to `.agents/RECORD.md`. Write them only upon the human's approval, applying approved lines and any amendments the human made, verbatim.
- Repository commands come from `steps/scm.md`. A missing command is a workflow bug — report it, don't improvise.
- A workflow file edited mid-run governs from the next action: re-read everything under this workflow's directory, migrate the standing attendants if their seeds changed (fresh load; retire old sessions by never messaging them again), append a cycle section to the ledger, and continue. A prompt update never restarts a run.

---

## Appendix A — the tracker

One file per issue in `.issues/` inside your checkout (untracked;
never committed; an existing one is this cycle's backlog):

```markdown
---
id = 1
title = "the issue in one line"
status = "triage"        # triage | open | contracting | dispatched | done | parent | closed
# parent = 1            # parent issue id; absent on top-level issues
# blocked-by = [2, 3]   # issue ids that must close first; absent when nothing blocks
# assignee = "<session-id>"  # absent when unassigned
---

# <title>
<goal, approach when known, verification>
```

- Statuses: `triage` (new) → `open` (claimable) → `contracting` → `dispatched` (an implementor holds it) → `done` (teardown succeeded). A decomposed issue becomes `parent` and closes when all children close.
- Claimable = `open` and every `blocked-by` id closed. Order by least blocked, then oldest.
- Bodies are written for a stranger and never mention the human; where work came from is ledger material.
- Keep frontmatter valid TOML — verify after every write.
- Sequential numbering is yours; you assign ids at file time.
- **Ids carry their title everywhere**: `003-fix-picker-focus`, never bare `003` — in the task list, every report, dispatch, verdict, and the ledger. The human reads those surfaces; the number alone is opaque. The slug is two to five kebab-case words from the title.

## Appendix B — the run ledger

`.autodev.ledger.md` at your checkout root, gitignored/fossil-ignored
at startup (see Startup step 2). Your private record across turns:
constants (branch, repo id, paths, workflow dir), every session id,
per-issue state transitions, review-round counts, the collector for
planner `record:` lines, and anything the QA brief's Known state must
name. Write as you go; the tracker is the system's record, the ledger
is yours. All three run-state paths (`.issues/`, `.plans/`, the
ledger) live inside the repo but can never be committed: the ignore
entry at startup guarantees it, and teardown demands clean status —
run state is never work.

## Appendix C — the implementor lifecycle

`lifecycle.toml` in this workflow defines one script per SCM
(`autodev implementor (fossil)` / `(git)`). Setup branches **your
branch** (the `<parent>` arg), opens the implementor's checkout, and
moves the session there. Teardown merges the work back onto your
branch, commits, closes the checkout, deletes the branch — the merge
is the landing; there is no integration branch.

Spawn procedure, per issue (steps/02-dispatch.md carries the full
payload lists):

1. `load_attendant` on `attendants/implementor.toml` (absolute path), with `set_name` = `implementor-<issue-number>` — the seed file's own name stays generic; the label distinguishes instances in the sidebar. Keep it short: `implementor-002`, never the issue slug. Never `spawn_subagent`.
2. `run_lifecycle` setup: `path` = this workflow's `lifecycle.toml` (absolute), `session_id` = the new session's id, `dir` = `<your-root>-issue-<id>` (sibling of your checkout), `branch` = `<your-branch>-issue-<id>`, `parent` = your branch.
3. `tell_session` the dispatch (issue path, contract path, ids, checkout dir, parent branch, scm.md path, report format).

Teardown: `run_lifecycle` teardown, session id only — recorded
coordinates replay it. The script gates on conflicts BEFORE any
irreversible step, so a `TEARDOWN-CONFLICT` exit changed nothing: send
the conflicted files to the same implementor (preserve — it knows the
code), it resolves/commits/reports, teardown again. Concurrent
implementors only when triage's file-territory fencing says disjoint;
same-file issues serialize because parallel teardowns would fight.

## Appendix D — the roles

- **Planner** (`attendants/planner.toml`, preserve, manual): one dispatch per issue before any implementor exists. Reads the issue, reads `.agents/RECORD.md` once (exploration fuel, not a constraint — the code is the authority), investigates, saves a contract to `.plans/<issue-id>-<slug>/contract.md` in your checkout: end state (checkable facts), orientation (how the touched code actually works — paths, functions, ordering), files to touch, approach, verification (exact commands), surprises & discoveries (findings with evidence). Reports `READY <issue-id>-<slug> <contract-path>` (with a `record:` line for stale/proposed record entries) or `BLOCKED` (issue contradicts itself, or end state unreachable — never capability doubt). Writes nothing else, edits no code.
- **Implementor** (`attendants/implementor.toml`, preserve, attached on demand, labeled `implementor-<issue-number>` via `set_name`): works its issue in its own checkout on its own branch, stamps the contract as its task list first, exploration bounded by the contract's files-to-touch list, verifies per the contract, commits on its branch. Reports `DONE` / `BLOCKED` with verification results and contract divergences. Never lands its own work, never touches the tracker, never spawns anything.
- **Reviewer** (`attendants/reviewer.toml`, preserve, manual): one dispatch per DONE report, before any teardown. Reads the contract then the branch diff through the repository file (no checkout). Judges: end state delivered? tests that can fail? scope matches files-to-touch? report honest? Reports `APPROVE <issue-id>-<slug>` / `REJECT <issue-id>-<slug>` + findings. Never edits anything.

Dispatches are always self-contained — issue path, contract path,
absolute paths, expected report format — paste paths, never retype
them. Re-dispatches (REJECT fixes, teardown conflicts) are shorter but
still explicit: what failed, the findings verbatim, what to do, same
report format.

## Appendix E — verification

- **Invariant greps** (run after any workflow edit, before any QA brief): `fossil`/`git` appear only in `lifecycle.toml` and `steps/scm.md`; "escalat" and "too hard" appear nowhere; the record file is written only on the human's approval of the QA brief's proposed entries.
- **Live checks**: the tracker parses (frontmatter TOML per file); every `dispatched` issue names a live session id; every `done` issue's branch is closed; your branch's log shows one merge per `done`.
- **QA brief** (`steps/04-qa.md`): what changed, how it should work (root contract's end state), how to test, known state, proposed record entries. Then stop.

## Appendix F — failure handling

- **Teardown conflict** → same implementor fixes (Appendix C). Never a tracker issue.
- **Teardown fails otherwise** → fix the invocation, re-issue. Stuck genuinely: issue stays `dispatched`, session named in the ledger and QA brief, loop continues.
- **Four consecutive REJECTs** → stop cycling: issue back to `open`, findings filed as a new issue, branch/checkout wait (teardown only follows APPROVE).
- **BLOCKED report** → facts filed as a new agent issue; the blocked session is abandoned (never messaged again); its checkout and branch are named in the QA brief's Known state. The issue re-enters the queue — being blocked once is not a demotion.
- **Silence** → hours past contract-sized effort with no report: abandoned, same as BLOCKED. There is no remote cancel; do not message it again.
- **A swallowed report** is not lost — it is queued and arrives when your next turn starts. Read the inbox before dispatching: review first, then hand out new work.
