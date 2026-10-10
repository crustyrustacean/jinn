# Workflow format — the full spec

Everything a workflow directory may contain, and every convention the
workflows skill applies when running one. Sketches here are shapes to copy,
not shippable workflows — no example ships with jinn.

## Directory layout

```
~/.config/jinn/workflows/<name>/
├── workflow.md
└── steps/
    ├── 01-scope.md
    ├── 02-research.md
    ├── 03-draft.md
    ├── 04-review.md
    └── 05-fix.md
```

- One directory per workflow, named in kebab-case (`research-report`).
- `workflow.md` is required; `steps/` is required (a workflow with no steps
  is a note, not a workflow).
- No frontmatter, no config files, no schema — a workflow is plain markdown.

## `workflow.md`

Three parts, in order:

1. **Header** — `# Name: <name>` then `# Description: <one line>`. The name
   must match the directory name.
2. **Flow** — an ordered list. One entry per step. Plain entries are prose
   phrases; two entry kinds are special and both are **explicit lines**:
   - `*(inline — conversation)*` — the step runs in the orchestrator
     session because its entire body is talking to the user.
   - Loop/branch lines, indented under the step they follow, written as
     `- If <condition> → <target step>, then back to <step> (repeat until
     <condition>)` or `- If <condition> → <target step>`.
3. **Parameters** (optional) — a short list of values the workflow needs at
   run time (a topic, a path, a depth). If present, each parameter names the
   step that consumes it.

The notation is fixed — one style, always written this way. Loop lines
always name both the target and the return edge; "repeat until" always
states its exit condition. A flow that cannot express its loop in this
notation is a sign the flow should be restructured, not that the notation
should grow.

### `workflow.md` field-by-field sketch

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

## Parameters

- topic — what to research; consumed by step 2.
- depth — "quick" or "thorough"; consumed by step 2.
```

## Step files

`steps/NN-slug.md` — zero-padded two-digit number, kebab-case slug, sorting
in execution order. Each file is a **complete prompt** for whoever executes
the step: assume the reader can see nothing else — not the workflow, not the
other steps, not the conversation that dispatched it.

Structure of a step file:

1. **Body** — the instructions. Where the step reads its inputs (parameters,
   files from earlier steps) and what it must produce.
2. **Output location** — steps whose output feeds a later step say where the
   output lands (a file path in the working directory is the default
   vehicle). A step whose output is short can end with it in the final
   message instead.
3. **Done when** — the last line, one sentence, stating how the runner knows
   the step finished: `Done when: research-notes.md exists and every claim
   in it carries a source.`

Two control conventions live inside step files, as plain lines:

- `stop` — if the step's situation makes the rest of the run pointless, the
  step reports `stop` in its final message; the orchestrator halts the run
  and tells the user.
- User confirmation — a step that must not proceed without the user's
  explicit yes ends by asking, and the orchestrator relays the answer at
  dispatch time. Confirmation is a convention recorded in prose, not a
  mechanism.

### Step-file sketch

```markdown
Review the draft against the agreed scope.

Input: draft.md (written by the previous step) and the scope note
(scope.md) from step 1.

Check: claims supported by the research notes; scope items all covered;
no invented sources.

Write your findings to review.md — a numbered list of issues, each naming
the draft section and what is wrong. If everything checks out, write
"clean" in review.md.

Done when: review.md exists and states either "clean" or a numbered list
of issues.
```

## Naming correspondence

The task name stamped on the task list is the **only** pointer to the step
file. The derivation, both directions:

- Stamp: `workflow.md` flow entry `3. Draft` → task name `3. Draft` →
  step file `steps/03-draft.md`.
- Derive: task name `4b. Review draft (round 2)` → strip the round
  annotation → `4. Review draft` → `steps/04-review.md`.

Rules that keep this derivable:

- Step filenames are `NN-<slug>.md` where the slug lowercases the task name
  and hyphenates it (`4. Review draft` → `04-review-draft.md`).
- Instance letters (`4b`) and round annotations (`(round 2)`) are list-side
  only — never part of a filename. Every iteration of a loop dispatches the
  same step file.
- Slugs are stable once a workflow has been run: renaming a step file breaks
  nothing mechanically, but a task name that no longer derives to a real
  file dead-ends the run.

## Stamping the task list

One `todo_set_list` call: a single phase named for the workflow, one task
per flow step in flow order. Conditionals are stamped as written and
resolved later; loops are stamped as their first round only.

Statuses and their meaning on a run:

- `pending` — not yet run. The run's position is the first pending task.
- `completed` — done-when met. Set the moment the step finishes.
- `cancelled` — a step the run skipped (a conditional branch not taken), or
  a step abandoned after a `stop`. Rewrite its name with `— skipped` or
  `— abandoned after stop` so the list reads as a record.

## Loop instances

A loop round is materialized as new task entries, added by the orchestrator
**before** the current round's last task is marked completed. After round 1
of the research-report loop produces issues:

```
Phase: research-report
1. Scope                        [completed]
2. Research                     [completed]
3. Draft                        [completed]
4. Review draft                 [completed]   (found issues — see 4b)
5. Fix issues                   [pending]
4b. Review draft (round 2)      [pending]
```

The next `todo_set_phase` rewrite adds only what exists: `5b. Fix issues
(round 2)` appears if round 2 also produces issues. The letter advances per
iteration of the loop, and the round annotation counts rounds. When a round
comes back clean, the loop simply stops generating instances — the entries
already on the list tell the whole story.

## Dispatch-prompt assembly

One `task` call per step. The prompt:

1. The step file's body, verbatim.
2. A parameter block for this run — every parameter the step consumes, with
   its value for this run.
3. The paths of prior steps' outputs the step reads.

Nothing else. In particular: do not paste prior steps' output into the
prompt — pass paths; do not summarize the workflow for the subagent — the
step file is the whole assignment. A dispatched subagent inherits the
working directory and tools but starts from an empty context and an empty
task list, and returns only its final message.

## Anti-patterns

- **Executing a step in the orchestrator session.** Researching, drafting,
  reviewing, or refactoring inline. Dispatch instead — the only inline work
  is conversation with the user.
- **Dispatching without the step file.** Writing the prompt from memory or
  from the task name alone. The file is the prompt; read it every time.
- **Pasting step output into dispatch prompts.** Bulky handoffs go through
  files; paths go through prompts.
- **Holding the loop in memory.** A round that produces another round must
  put the next instance on the task list before the round closes. A loop
  that exists only in the orchestrator's head does not survive a compacted
  context and is invisible to the user.
- **Silent skips.** A branch not taken must land on the list as
  `— skipped`/`cancelled`, not quietly vanish.
- **Renaming step files mid-run** or inventing slugs at dispatch time. The
  derivation rule is the loader.
- **Growing the format.** Adding schema fields, a manifest, or status
  vocabulary a runner would have to implement. The format is markdown and
  naming conventions; its executor is a disciplined session.
