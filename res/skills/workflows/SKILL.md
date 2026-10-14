---
name: workflows
description: Author and run jinn workflows — named multi-step procedures stored as markdown under ~/.config/jinn/workflows/, stamped onto the session task list and executed while the session orchestrates. Covers both execution architectures (sequential step dispatch via `task`, and event-driven session pools via attendants) and the harness primitives they ride (`task`, attendants, `tell_session`/`notify_parent`, lifecycle scripts). Use when the user asks to run a workflow, author or create one, or describes a repeatable multi-step procedure (draft-review-fix cycles, research pipelines, staged refactors, autonomous development loops) to be carried out step by step.
---

# workflows — author and run multi-step procedures

A **workflow** is a named, reusable procedure: one directory under
`~/.config/jinn/workflows/<name>/` holding a `workflow.md` (the flow, with
loops and conditionals spelled out) and a `steps/` directory of prompt files.
Nothing runs by itself — a workflow is inert markdown. Running one is a
discipline the session applies.

This skill covers the whole lifecycle: author a workflow from a prose
description (orient → one batched question round → user-approved preview →
write), stamp it onto the task list, run it, close it out. Depth lives in
references; the must-knows are in this file.

## Pick the architecture first

A workflow is either **sequential** or **event-driven**, and the choice is
the first authoring decision — it changes the directory layout, the
execution model, and what the session must supervise.

- **Sequential** — steps run one after another; each step is one unit of
  work; when it's done, the next starts. Use when the flow is a pipeline:
  research → draft → review → fix → deliver. Execution: the orchestrator
  dispatches each step to a fresh subagent with the `task` tool.
- **Event-driven** — a set of named worker sessions runs concurrently and
  reacts to events: messages between sessions, turn completions, task
  completion. Use when the flow has concurrency, recursion, or long-lived
  workers: fanout, an issue queue that spawns more issues, workers that
  report back asynchronously. Execution: the orchestrator loads
  **attendants** (workflow-local session seeds), dispatches them with
  `tell_session`, and receives their reports via `notify_parent`.

When both fit, prefer sequential — it is simpler to author and to debug.
Event-driven earns its cost only when the flow genuinely needs
concurrency or recursion. The worked examples: `res/workflows/autodev/`
is the reference event-driven workflow (an autonomous development loop —
the most sophisticated workflow that ships with jinn); the sketches in
`references/workflow-format.md` are the reference sequential shape.

## The harness primitives (both architectures)

These are the tools a workflow run can rely on. Know what each one
actually does — authoring against a wrong mental model is the main way a
workflow dies on first contact.

- **`task`** — spawn a one-shot subagent that runs a single prompt to
  completion and returns only its final message. **Blocking**: the call
  does not return until the subagent finishes. Subagents are isolation,
  not compression: they start from an empty context, see nothing but the
  prompt, and are the right tool for bounded, self-contained steps.
- **Attendants** — standing child sessions defined by a TOML seed file.
  `trigger = "manual"` attendants run only when messaged;
  `behavior = "reset"` gives each dispatch a fresh context (pinned
  instructions only); `behavior = "preserve"` keeps the conversation so
  later dispatches reuse earlier exploration. An attendant cannot create
  attendants, cannot run lifecycle scripts on itself mid-run, and has
  exactly one inbound channel (its parent) and one outbound tool
  (`notify_parent`).
- **`tell_session` / `notify_parent`** — the inter-session channels.
  Messaging an idle attendant starts a run seeded from its template plus
  your message; messaging a busy one **queues** the message behind its
  current turn — nothing is ever cancelled, nothing races. An attendant's
  `notify_parent` starts a turn in its parent, **if the parent is
  idle** — a busy parent has it queued for its next turn. This
  turn-boundary delivery is why event-driven orchestrators end their
  turns: **a turn that never ends is a turn whose reports never land.**
- **`conclude`** — an attendant records one summary per run into a
  history log. Its text re-enters context **only** through the
  `<prior report>` token in the seed template. A template without the
  token never sees reports; do not put the token in a seed whose
  attendants would receive out-of-order conclusions (e.g. a reviewer
  judging many issues in arbitrary order).
- **Session lifecycle scripts** — a TOML of setup/teardown commands,
  run via `run_lifecycle` with the script's name and file path.
  **Setup is one-shot per session**: the tool refuses a second setup, so
  a workflow gets exactly one scripted environment per session and must
  spend it at startup. Setup adopts the last path the script prints as
  the session's working directory — this is how a session "moves into"
  its own checkout or worktree. Placeholders (`<dir>`, `<branch>`) are
  filled from named args, shell-quoted.
- **`save_plan`** — persists a plan file that survives context
  compaction. The right vehicle for any artifact a later dispatch must
  read (contracts, specs, decompositions).

## The laws (hold in every architecture)

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
5. **Bound the fanout.** Concurrent workers are a declared constant of
   the workflow (a pool size), not an open door. Every recursion has a
   structural ceiling: only the orchestrator spawns, and only from a
   fixed set of seed files.
6. **No silent failure.** A report format (DONE/BLOCKED, APPROVE/
   REJECT) is part of every dispatch, and every non-success report is
   routed somewhere: re-dispatch, re-triage, or a filed issue. A
   message that arrived and changed nothing is a bug in the workflow.
7. **The form factor is settled.** The user asked for a workflow; a
   workflow is the deliverable even when one step would do. The only
   open question is which architecture — and that one is asked in the
   preview round, not re-litigated after.

## Authoring

Authoring follows one protocol, ending in a **preview the user approves
before anything is written to disk**. The user invoked the skill, so the
output is always a workflow — judgment about the flow is the job;
judgment about _whether_ it should be a workflow is not.

### 1. Orient (only when you have to)

Before composing questions, ask: **can I form the right questions about
this domain?** A README for a code repo — yes; skip. An AI voiceover
pipeline — no: run a few cheap searches first so your questions carry
concrete options instead of blank asking. Minutes, not an hour.

### 2. Ask (one round, batched, hard-capped)

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

### 3. Preview (always)

Show the workflow in chat **before writing anything** — name,
architecture, the flow list with loop/branch lines, parameters with
defaults, one line per step file, and — for event-driven workflows —
the session topology (who exists, who talks to whom, the fanout bound).
Readable in under a minute. End with: approve, or say what to change.

**Every judgment call surfaces here or never.** Choice-shaped facts —
defaults picked, an instruction baked into a step, an extra you
considered — appear in the preview or they are not decisions. After
approval there is nothing left to choose.

### 4. Write — and stop

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

## Running

Read `references/workflow-format.md` (sequential) or
`references/event-driven-workflows.md` (event-driven) before your first
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
  orchestrator's constitution); step files are the per-stage procedures
  it names. End your turn whenever nothing is left to do this instant —
  turn-end is what delivers queued reports. Workers wake you; you are
  the only session that never waits idly.
- **Model pinning**: pass `model` on a `task` call only when the user
  explicitly pinned a step to a model. Never to pick one you prefer.

## Where to look next

| File                                       | Covers                                                                                                                                                                              |
| ------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `references/workflow-format.md`            | Sequential architecture, full spec: `workflow.md` fields, step-file anatomy, loop/conditional notation, naming rules, the authoring dialogue expanded, dispatch-prompt assembly, anti-patterns |
| `references/event-driven-workflows.md`     | Event-driven architecture: session topology, attendant seed files (behavior, trigger, pins, templates), the turn-boundary wake model, lifecycle budgets, state files, the orchestrator's constitution pattern, failure and stall handling, anti-patterns |
| `res/workflows/autodev/`                   | The shipped worked example — event-driven: orchestrator constitution, five stage procedures, three attendant seeds (planner/implementor/reviewer), an SCM crib sheet, lifecycle scripts. Read it before authoring anything with concurrency or recursion |
