---
name: workflows
description: Author and run jinn workflows — named multi-step procedures stored as markdown under ~/.config/jinn/workflows/, stamped onto the session task list and executed while the session orchestrates. Covers both execution architectures (sequential step dispatch via `task`, and event-driven session pools via attendants) and the harness primitives they ride (`task`, attendants, `tell_session`/`notify_parent`, lifecycle scripts). Use when the user asks to run a workflow, author or create one, or describes a repeatable multi-step procedure (draft-review-fix cycles, research pipelines, staged refactors, autonomous development loops) to be carried out step by step.
---

# workflows — author and run multi-step procedures

A **workflow** is a named, reusable procedure: one directory under
`~/.config/jinn/workflows/<name>/` holding a `workflow.md` (the flow,
with loops and conditionals spelled out) and a `steps/` directory of
prompt files. Nothing runs by itself — a workflow is inert markdown.
Running one is a discipline the session applies.

## First: which job is this?

Determine your job before reading further — the two share vocabulary but
not procedure:

- **RUN** — a workflow already exists (installed under
  `~/.config/jinn/workflows/`, named by the human, or launched via
  `<leader>sw`) and this session is to execute it as the orchestrator.
  The directory is on disk; do not modify it mid-run. → Read
  [Running](#running) below, then the reference for the workflow's
  architecture (its `workflow.md` will make the architecture obvious:
  a flow list of one-shot steps = sequential; stage procedures plus
  attendant seeds = event-driven).
- **AUTHOR** — the human describes a procedure they want to exist, or
  asks to create/change a workflow. The deliverable is the directory
  itself. → Read [Authoring](#authoring) below (which includes picking
  the architecture), then the reference for the architecture you
  picked.
- **Both at once** — "make one and run it" is two jobs in sequence:
  author (through user approval), then run. Never collapse the
  approval gate because the run is coming anyway.

When unsure, ask one question: "run an existing workflow, or create
one?" — never author as a side effect of being asked to run, and never
launch a run as a side effect of authoring (that choice is the human's
next message, not yours).

---

## Authoring

Authoring follows one protocol, ending in a **preview the user approves
before anything is written to disk**. The user invoked the skill, so the
output is always a workflow — judgment about the flow is the job;
judgment about _whether_ it should be a workflow is not.

### 1. Pick the architecture

A workflow is either **sequential** or **event-driven**, and the choice
is the first authoring decision — it changes the directory layout, the
execution model, and what the session must supervise.

- **Sequential** — steps run one after another; each step is one unit of
  work; when it's done, the next starts. Use when the flow is a pipeline:
  research → draft → review → fix → deliver. Execution: the orchestrator
  dispatches each step to a fresh subagent with the `task` tool.
- **Event-driven** — named worker sessions run concurrently and react
  to events: messages between sessions, turn completions, task
  completion. Use when the flow has concurrency, recursion, or
  long-lived workers: fanout, an issue queue that spawns more issues,
  workers that report back asynchronously. Execution: the orchestrator
  spawns **attendants** (workflow-local session seeds), dispatches them
  with `tell_session`, and receives their reports via `notify_parent`.

When both fit, prefer sequential — it is simpler to author and to debug.
Event-driven earns its cost only when the flow genuinely needs
concurrency or recursion. The worked examples: `res/workflows/autodev/`
is the reference event-driven workflow (an autonomous development loop —
the most sophisticated workflow that ships with jinn); the sketches in
`references/workflow-format.md` are the reference sequential shape.

### 2. Orient (only when you have to)

Before composing questions, ask: **can I form the right questions about
this domain?** A README for a code repo — yes; skip. An AI voiceover
pipeline — no: run a few cheap searches first so your questions carry
concrete options instead of blank asking. Minutes, not an hour.

### 3. Ask (one round, batched, hard-capped)

Put **every question in one message**, then build regardless of what
happens. What goes in it:

- **Structural forks as lettered options** — only when the flow's shape
  genuinely depends on the answer. The **architecture choice**
  (sequential vs event-driven) is the most common one; each option
  states what it does, what it costs, and which you recommend in one
  sentence.
- **Environment prerequisites.** Anything the workflow cannot survive
  without that you cannot verify from the prompt: API keys, accounts,
  which SCM the target repo uses, local installs.
- **The steps they wouldn't have thought to name.** Your domain
  knowledge is the value here. Always include this list when you know
  of candidates, even if the user enumerated their steps.

What does not go in the round: parameter gaps with sensible defaults
(assume; the default becomes a named parameter in the preview);
anything the prompt already answered.

### 4. Preview (always)

Show the workflow in chat **before writing anything** — name,
architecture, the flow list with loop/branch lines, parameters with
defaults, one line per step file, and — for event-driven workflows —
the session topology (who exists, who talks to whom, the fanout bound).
Readable in under a minute. End with: approve, or say what to change.

**Every judgment call surfaces here or never.** Choice-shaped facts —
defaults picked, an instruction baked into a step, an extra you
considered — appear in the preview or they are not decisions. After
approval there is nothing left to choose.

### 5. Write — and stop

After approval — and only after — create the directory under
`~/.config/jinn/workflows/<name>/` with the `write` tool (see the
references for the per-architecture layout). Then one closing sentence
naming what was created, and stop. No follow-up menu: running it,
tweaking it — those are the user's next message, not an offer.

### Pre-approval

"Just make it," "don't ask" — the user has pre-approved. Collapse the
question round and the preview: build with your best defaults, write,
report what you made in the closing summary. Silence after a question
is _not_ pre-approval.

---

## Running

You are the orchestrator. The workflow's files govern; your judgment
supplements them, never overrides. Read the architecture reference
(`references/workflow-format.md` sequential,
`references/event-driven-workflows.md` event-driven) before your first
run of that architecture. The irreducible core:

- **Stamp the task list** — one phase, one task per flow step (or per
  flow stage), updated the moment a decision or result lands. Loop
  iterations are new task instances, added before the current round
  closes.
- **Read the step file every time** — the file is the prompt; never
  dispatch from memory.
- **Sequential**: one `task` call per step, prompt = step file body +
  parameter values + input paths. Check the step's done-when against
  the returned message; re-dispatch once with the shortfall named on a
  miss; stop after a second miss.
- **Event-driven**: the loop body lives in `workflow.md` (the
  orchestrator instructions); step files are the per-stage procedures
  it names. End your turn whenever nothing is left to do this instant —
  turn-end is what delivers queued reports. Workers wake you; you are
  the only session that never waits idly.
- **Model pinning**: pass `model` on a `task` call only when the user
  explicitly pinned a step to a model. Never to pick one you prefer.

Close the run by reporting to the user: what was produced (paths), and
the loop count if the flow iterated.

## Rules (hold in every architecture, both jobs)

1. **You orchestrate; you do not execute.** Working material — research,
   drafts, code, reviews — belongs in dispatched sessions, never in the
   orchestrator's context. The one exception: a step whose entire body
   is conversation with the user.
2. **Dispatches are self-contained.** The receiver sees the prompt and
   the filesystem, nothing else. Every dispatch carries: what to do
   (the step file or contract), the absolute paths of everything it
   reads and writes, and the expected report format. Paste paths,
   never retype them.
3. **State lives in files, not in memory.** Anything that must survive
   a context compaction, a turn boundary, or a session restart is a
   file with its path recorded in a known place. The orchestrator's
   memory is not a queue, not a ledger, not a cache.
4. **The task list is the run's visible state.** Stamp it at start,
   update it the moment anything happens — never batched. A reader
   watching the list should be able to reconstruct where the run is.
5. **Bound the fanout.** Concurrency is bounded by the work actually
   claimable, and every recursion has a structural ceiling: only the
   orchestrator spawns, only from a fixed set of seed files, and every
   spawned session is created for a specific dispatch — never spawned
   into idleness "for later."
6. **No silent failure.** A report format (DONE/BLOCKED, APPROVE/
   REJECT) is part of every dispatch, and every non-success report is
   routed somewhere: re-dispatch, re-triage, or a filed issue. A
   message that arrived and changed nothing is a bug in the workflow.
7. **The form factor is settled.** The user asked for a workflow; a
   workflow is the deliverable even when one step would do. The only
   open question is which architecture — and that one is asked in the
   preview round, not re-litigated after.
8. **Respect the job boundary.** Authoring ends at the write (plus one
   closing sentence); running starts at the existing directory. An
   authoring session does not launch the workflow it just wrote, and a
   running session does not edit the workflow it is running.

## Where to look next

| File                                       | Covers                                                                                                                                                                              |
| ------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `references/workflow-format.md`            | Sequential architecture, full spec: `workflow.md` fields, step-file anatomy, loop/conditional notation, naming rules, the authoring dialogue expanded, dispatch-prompt assembly, anti-patterns |
| `references/event-driven-workflows.md`     | Event-driven architecture: session topology, attendant seed files (behavior, trigger, pins, templates), the turn-boundary wake model, lifecycle scripts (setup/teardown as the whole checkout contract), state files, the orchestrator instructions pattern, failure and stall handling, anti-patterns |
| `res/workflows/autodev/`                   | The shipped worked example — event-driven: orchestrator instructions, five stage procedures, three attendant seeds (planner/implementor/reviewer), an SCM crib sheet, lifecycle scripts (on-demand implementors; setup branches the feature branch, teardown merges back; contracts record a pre-existing-failure baseline that gates measure against; work-item ids carry title slugs everywhere). Read it before authoring anything with concurrency or recursion |
