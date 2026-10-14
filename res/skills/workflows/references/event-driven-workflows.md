# Event-driven workflows — sessions that react, not steps that follow

The second jinn workflow architecture. Where a sequential workflow is a
pipeline of one-shot subagents, an event-driven workflow is a **topology of
standing sessions** that exchange messages and react to completion events:
an orchestrator session, a bounded pool of workers, and any number of
specialist attendants. Use it when the flow needs concurrency, recursion,
or long-lived workers; use the sequential architecture otherwise.

**`res/workflows/autodev/` ships as the worked example** — an autonomous
development loop with a planner, a reviewer, and a three-worker pool. This
file states the general rules; that directory shows all of them in
working code. Read both before authoring your first event-driven
workflow.

## The topology

Every event-driven workflow has the same shape:

```
                 ┌── tell_session ──▶ planner (1)
   orchestrator ─┼── tell_session ──▶ reviewer (1)
                 └── tell_session ──▶ workers (K)
        ▲                     │
        └──── notify_parent ──┘        (all reports ride one channel)
```

- **One orchestrator** — the human-facing session that started the
  workflow. It holds the task list, the ledger, and every decision. It
  is the only session that talks to every other session.
- **Attendants** — child sessions, each loaded from one TOML seed file.
  Workers (the fanout pool, loaded K times from one seed), and
  specialists loaded once each (planner, reviewer, any other role whose
  procedure is too big to be dispatch data).
- **Hub-and-spoke messaging.** Every arrow goes through the
  orchestrator. Attendants learn only their parent's id (the harness
  appends it to every dispatch); sibling ids never enter their context,
  and dispatch payloads carry file paths, not session ids. Attendants
  cannot create attendants. If two attendants seem to need to talk,
  the workflow is missing an orchestrator step — route it through the
  hub.

The fanout bound is structural: only the orchestrator spawns, only from
the workflow's fixed set of seed files, and the pool size is a declared
constant. An orchestrator that loads a fifth worker is broken; an
attendant that spawns anything is impossible.

## The wake model — how work actually moves

Sessions cannot start themselves. Everything moves through two
mechanisms, and both are turn-boundary:

1. **`tell_session`** (orchestrator → attendant): an idle attendant
   starts a run immediately, seeded from its template plus the message;
   a busy one queues the message and runs it when its current turn
   ends. Nothing cancels; nothing races. One queued message is the most
   a worker should ever hold — the tracker/state files hold the real
   queue.
2. **`notify_parent`** (attendant → orchestrator): lands as a user
   message that starts the parent's next turn. If the parent is
   mid-turn, the report **queues until that turn ends**.

The consequence that shapes every event-driven orchestrator: **end your
turn.** After dispatching, or whenever nothing is left to do this
instant, the orchestrator ends its turn — that is what lets the next
report land. A turn held open with sleeps is a turn whose queued
reports never arrive; a turn that never ends is a wedged run. The
orchestrator's turn ends are not pauses to be optimized away; they are
the delivery mechanism.

A scan timer, if the workflow wants one, is plain shell inside the
turn — sleep, rescan state files, act, end turn — not a held turn.
Its only job is detecting silence (a worker that went quiet with work
in flight); events do the actual waking.

## Attendant seed files

One file per role, TOML, living beside the workflow (loaded by absolute
path via `load_attendant`; the same shape as an `[[attendant.entry]]`
block in `jinn.toml`, but workflow-scoped — it cannot be saved to user
config). Load one file **K times to get K workers of that role**; a
pool is one seed loaded repeatedly, not K files.

```toml
[[attendant.entry]]
name = "implementor"
behavior = "reset"          # reset | preserve — context across dispatches
trigger = "manual"          # ALWAYS manual for dispatch-driven roles
prep_mode = false
seed_template = "Your last run reported: <prior report>"
pins = [{ role = "user", text = """ ...standing instructions... """ }]
```

Fields that matter:

- **`trigger`** — `"manual"` for anything the orchestrator dispatches.
  A completion trigger fires on every parent turn end, which is the
  opposite of dispatch-driven work: a manual-trigger attendant runs
  only when messaged, so idle workers cost nothing.
- **`behavior`** — `"reset"` wipes context at each dispatch: the run
  sees its pins plus the one dispatch, so dispatches must be
  self-contained. Right for fungible workers. `"preserve"` keeps the
  conversation, so the session's exploration (a repo map, a codebase
  orientation) survives across dispatches. Right for specialists whose
  value compounds — and it comes with a hygiene rule: state in the pin
  what persists (geography, maps) and what must not (verdicts,
  judgments about past work), or the session starts assuming from its
  own history.
- **`seed_template`** — the standing text prepended to every dispatch.
  The `<prior report>` token is the **only** way a `conclude` report
  re-enters context. Put the token in a template only when the
  attendant's conclusions arrive in an order that makes the previous
  one relevant (one work unit per run: a worker). Leave it out
  otherwise (a reviewer judging many issues in arbitrary order) — a
  template without the token never receives reports. A template with
  no token still needs its own text: it is what the attendant reads
  first on every dispatch.
- **`pins`** — the attendant's standing instructions: role, what it
  must never do, how it reports. Written for a stranger; every
  dispatch assumes the pins are all the context there is (reset) or
  the durable part of it (preserve).

A seed's pins should always carry: the role in one sentence, the hard
boundaries (never touch the tracker / never write product code / never
create sessions), the procedure or artifact it produces, and the exact
report format — including that it reports via `notify_parent` and never
lets information travel any other way.

## One-shot lifecycle budgets

`run_lifecycle` setup is **one per session, ever**. An event-driven
workflow therefore spends its budget at startup and only at startup:

- The orchestrator spends its one setup moving itself into its working
  checkout (autodev's `integration-open`).
- Each worker's setup is issued **by the orchestrator, targeting the
  worker's session id**, immediately after loading it — the worker
  wakes up already sitting in its own checkout (autodev's
  `worker-open`).
- Specialists that need no filesystem seat (readers who work through
  paths, or through a repository file with read-only SCM commands) get
  no setup — saving the budget is the point of giving them no checkout.

Anything a session needs *later* — a fresh checkout for a one-shot task,
a teardown — is plain shell in a step file, often in a subshell so the
session never leaves its seat. The lifecycle TOML and one crib-sheet
step file are the only places a workflow should name SCM commands;
every other file stays SCM-blind so porting to another SCM touches two
files.

## State files

Event-driven workflows accumulate state that outlives any turn. Keep it
in files, and name each one's owner and purpose in the workflow:

- **The ledger** (orchestrator's private record, beside its checkout):
  the run's constants, paths, session ids, decisions, and things to
  carry into the QA brief. The orchestrator's memory across turns.
- **The tracker** (the system's shared record): the work items
  themselves — one file each, frontmatter for state, body for the
  work. The tracker is the queue, the dependency graph, and the
  run's history. Declare its location, its frontmatter schema, and
  **its single writer** — the orchestrator writes transitions; workers
  never touch it (they report through `notify_parent` and the
  orchestrator files the consequences).
- **Artifacts** (contracts, plans, briefs): written by whoever owns
  them, passed by path. `save_plan` for anything that must survive
  compaction.

Single-writer is what makes file-based coordination safe in a
multi-session run. Every state file gets exactly one owner; everyone
else reads or requests.

## The orchestrator's constitution

A sequential workflow's `workflow.md` is a flow list; an event-driven
one is a **constitution**. It must carry, in order:

1. **Frontmatter** — same as always (`name`, `description`; discovery
   reads nothing else).
2. **Constants** — the declared numbers and names: pool size K, branch
   names, tracker and artifact paths, stall thresholds. Every number a
   session could otherwise improvise.
3. **Startup** — the ordered setup: probe the seat, purge stale state,
   file the goal, spend the lifecycle budget, load the attendants
   (which file, how many times, in what order), stamp the task list.
4. **The loop** — numbered stages naming their step files, with the
   turn-end rule stated explicitly: end the turn after dispatching and
   whenever nothing is left to do; reports wake you.
5. **Amendment** — how the run behaves when the human edits the
   workflow mid-run: re-read everything, migrate the pool (load new
   seeds, retire old sessions by never messaging them again, reuse
   their seats), append a cycle section to the ledger, and continue —
   a prompt update is never a restart.
6. **Boundaries** — who writes what, what no session may do, what
   gates exist (and the statement that none exist beyond the declared
   human contacts).

## Steps, reports, and routing

- Step files are per-stage procedures the constitution names — written
  for the orchestrator executing them, not for a stranger subagent
  (the attendants get their procedures through their seed pins and
  their dispatches).
- Every dispatch states the **report format** the orchestrator will
  parse (autodev: `DONE <issue-id> <branch>` / `BLOCKED <issue-id>`;
  `READY <issue-id> <contract-path>`; `APPROVE|REJECT <issue-id>`).
  Short, greppable, first-line-parseable.
- Every non-success report routes somewhere explicit: a BLOCKED report
  files its facts as new work items; a REJECT re-dispatches the same
  worker with the findings verbatim; a stall re-queues the work.
  Nothing ever just disappears, and the orchestrator never does the
  worker's job itself to unblock a stage.
- **No escalation paths.** If the flow must assume attendants can do
  what is asked, say so in the pins and the constitution: a dead end
  is reported as facts (BLOCKED), not as a request for a human. The
  orchestrator files the facts and re-sequences.

## Failure and stall handling

The harness gives an event-driven workflow no remote cancel, no
attention signal, and no watchdog for wedged turns. Design for it:

- **Stalls**: a worker silent for N scan windows is abandoned — its
  work item returns to the queue, the pool absorbs the work, the
  orphan is named in the next human report. The orchestrator never
  messages a suspected-dead worker again this run; its session just
  goes idle forever.
- **Non-convergence**: a work item bounced back and forth past a
  declared bound (autodev: four consecutive REJECTs) leaves the cycle —
  back to the queue with the findings filed as a new item, so the
  tracker re-triages it instead of the same two sessions arguing
  forever.
- **Queued-message hazards**: a message queued behind a cancelled turn
  can land as an editable draft in the target session's input box.
  Design dispatch payloads so a stale draft is safe (self-contained,
  dated by content, idempotent to re-run), and name the hazard in the
  orchestrator's amendment section.
- **A failing run is data.** Session transcripts, the tracker, and the
  ledger are readable after the fact (jinn's own session-search tools
  or the session store). Do not clean up a failed run before reading
  it.

## Anti-patterns

- **Attendant-to-attendant messaging.** Sibling ids never enter
  context; routes go through the orchestrator. A workflow that wants
  workers to coordinate directly is missing an orchestrator stage.
- **The held turn.** Sleeping in a loop to "wait" for reports. Turn-end
  is the delivery mechanism; a turn that never ends never receives
  them.
- **A pool of duplicate files.** K workers is one seed loaded K times.
  Three copies of one TOML drift.
- **The preserve everything seed.** Preserve without a
  geography-not-verdict hygiene rule turns a specialist into a
  grudges-by-email machine: it starts judging from its history instead
  of the work in front of it.
- **Completion triggers on dispatch-driven attendants.** The attendant
  fires on every orchestrator turn end and invents work. Manual
  trigger, always, for anything the orchestrator dispatches.
- **The improvise number.** A pool size, retry count, or timeout that
  lives in the orchestrator's judgment instead of the constitution's
  constants. Every number is data.
- **Tracker writes from workers.** Single-writer is what makes the
  file-based queue safe. A worker that edits its own issue file is a
  race the tracker cannot survive.
- **The silent second queue.** Holding undelivered work in session
  inboxes instead of state files. The tracker holds the queue; inboxes
  hold at most one message.
- **Bounded fanout as a suggestion.** "A few workers, as needed" is
  unbounded. K is a constant; recursion is structural (only the
  orchestrator spawns, from a fixed file set).
