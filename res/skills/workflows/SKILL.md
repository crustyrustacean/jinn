---
name: workflows
description: Author and run jinn workflows — named multi-step procedures stored as markdown under ~/.config/jinn/workflows/, stamped onto the session task list and executed by dispatching each step to a subagent while the session orchestrates. Use when the user asks to run a workflow, author or create one, or describes a repeatable multi-step procedure (draft-review-fix cycles, research pipelines, staged refactors) to be carried out step by step.
---

# workflows — author and run multi-step procedures

A **workflow** is a named, reusable procedure: one directory under
`~/.config/jinn/workflows/<name>/` holding a `workflow.md` (the flow, with
loops and conditionals spelled out) and a `steps/` directory of prompt files,
one per step. Nothing runs by itself — a workflow is inert markdown. Running
one is a discipline the session applies, using only the ordinary task tools
and the `task` dispatch tool.

This skill covers the whole lifecycle: author a workflow from a prose
description, stamp it onto the task list, run it as an orchestrator, and
close it out. `references/workflow-format.md` is the full format spec.

## You are the orchestrator

When a workflow runs, your session **orchestrates; it does not execute**.
The division of labor:

- **You** — read the workflow, maintain the task list, load each step's
  prompt file, dispatch it to a subagent with the `task` tool, check the
  result against the step's done-when line, update the list, move on.
- **Subagents** — do the actual work of each step, in their own fresh
  context. They see nothing but the prompt you send.

**Running steps in your own context is the failure mode this skill exists to
prevent.** If you catch yourself researching, drafting, reviewing, or
refactoring *inside the orchestrator session*, stop and dispatch instead.
Why it matters: the step's working material floods your context and degrades
everything after it; the user cannot see per-step progress; steps lose their
isolation from each other; and the task list stops meaning anything, because
tasks get marked done with no visible work behind them.

**The tiebreak**: *would doing this fill my context with the step's working
material?* Any yes → dispatch. The one exception: a step whose entire body
is conversation with the user (asking for the topic, confirming a choice)
runs inline — there is nothing to isolate.

## Authoring

A prose description is enough to author a workflow. Write the directory with
the `write` tool — no scaffold, no template, no command:

```
~/.config/jinn/workflows/<name>/
├── workflow.md          # header + the flow as an annotated list
└── steps/
    ├── 01-scope.md      # one self-contained prompt per step
    ├── 02-research.md
    └── ...
```

`workflow.md` states the name, a one-line description, then the flow as an
ordered list where **loops and conditionals are explicit lines, not
prose hints**. `steps/NN-slug.md` files sort in execution order (zero-padded
numbers) and each one is a complete prompt for whoever executes the step —
assume the reader is a subagent that can see nothing else. Sketch of a
`workflow.md` (not a shippable example — just the shape):

```markdown
# Name: research-report
# Description: Research a topic and deliver a cited brief.

## Flow

1. Scope — ask the user for the topic and depth. *(inline — conversation)*
2. Research the topic.
3. Draft the brief from the research notes.
4. Review the draft against the scope.
   - If the review found issues → step 5, then back to step 4 (repeat until clean).
   - If the review was clean → step 6.
5. Fix the issues the review found.
6. Deliver the brief. *(inline — conversation)*
```

And one step file, `steps/02-research.md`:

```markdown
Research the given topic to the requested depth.

Parameter: topic — provided at dispatch.
Parameter: depth — "quick" or "thorough".

Write your findings to research-notes.md in the working directory: a section
per theme, each claim with its source URL.

Done when: research-notes.md exists and every claim in it carries a source.
```

The full field-by-field spec, the loop/conditional notation, and naming
rules are in `references/workflow-format.md`. Read it before your first
authoring pass; after that, the shapes above are the whole format.

## Stamping

Instantiating a workflow means writing its steps onto the task list with
`todo_set_list` — one task per step, one phase for the run (named after the
workflow). **The task name is the pointer to the step file**: strip the
number and annotations, lowercase, spaces to hyphens — `2. Research` →
`steps/02-research.md`. Nothing else carries the pointer; keep the names
corresponding.

Stamp the flow as written, resolving nothing early:

```
Phase: research-report
1. Scope                4. Review draft
2. Research             5. Fix issues
3. Draft                6. Deliver brief
```

Then the loop and conditional conventions, applied while the run is live:

- **Loop iterations are instances.** When a round produces another round,
  **you add the next instance to the task list yourself** (a `todo_set_phase`
  rewrite) *before* marking the current one done — the loop exists on the
  list, not in your memory. Instance naming: step number + letter + round,
  e.g. after round 1's review finds issues, add `4b. Review draft
  (round 2)` and `5b. Fix issues (round 2)`.
- **Conditionals resolve in place.** When a branch decision lands, record it
  on the list: a taken branch becomes ordinary instances; a skipped step is
  rewritten as e.g. `5. Correction pass — skipped` with status `cancelled`.
- The task tools know three statuses: `pending`, `completed`, `cancelled`.
  There is no in-progress; the run's position is simply the first pending
  task. Flip a task to `completed` the moment its step's done-when is met.

## The dispatch loop

For each task, in order:

1. **Read the step file** (`steps/NN-slug.md`, derived from the task name)
   with the `read` tool.
2. **Dispatch it** with one `task` call. The prompt is the step file's body
   verbatim, plus: parameter values for this run, and the pointer (a path)
   to anything a prior step left behind. That is all the subagent gets — it
   cannot see the workflow, the task list, or your conversation, so nothing
   may live only in your head. Keep bulky handoffs in files: a step whose
   output is more than a few sentences writes it to a file and ends with the
   path; you pass the path to the next step.
3. **Check done-when.** The step file's `Done when:` line says how you know
   it finished. The subagent's final message must satisfy it (a path exists,
   a question was answered). If the step signals `stop`, halt the run and
   report to the user. If done-when is not met, re-dispatch the same step
   file once with the shortfall named in the prompt; if it fails again, mark
   the task `cancelled` and stop — do not improvise the step yourself.
4. **Update the list** — mark the task `completed`; if the step's outcome
   re-enters a loop, add the next instance first (above).
5. **Move on** to the next pending task. Steps dispatch sequentially; run
   them in the list's order.

Steps marked *(inline — conversation)* you handle yourself, in seconds, and
then flip to `completed` like any other task: ask, get the answer, record it
as a parameter for the next dispatch. Do not let an inline step grow work.

Close the run by reporting to the user: what was produced (paths), and the
loop count if the flow iterated.

## Where to look next

| File | Covers |
|---|---|
| `references/workflow-format.md` | Full format spec: `workflow.md` fields, step-file anatomy, the loop/conditional notation, naming rules, dispatch-prompt assembly, anti-patterns |
