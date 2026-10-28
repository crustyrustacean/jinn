# Builtin Prompts

Typing `#` in input mode opens the prompt popup. Each builtin is a markdown
template that jinn replaces with its full body when the message is sent (the
model sees the body, never the `#token`). Anything typed after the token is
appended after the template body — each template ends with a marker heading
(`## SUBJECT`, `## TASK`) so your text lands where the template expects it.

This is a guide to what each builtin is for and how it works. For expansion
mechanics and writing your own templates, see `chat-input-tokens.md` and
`extending-jinn.md`.

## The analysis prompts

`#diverge`, `#rank`, `#judge`, and `#falsify` share one design: the agent does
not answer the question itself in that turn. It writes a short brief, spawns
parallel subagents with the `spawn_subagent` tool, and reports their answers.
The subagents never see the conversation, so their answers are independent.
These prompts do not change anything — they are read-and-report turns, and
each report leads with the answer in one sentence.

| Prompt     | Question it answers     | Subagents                             | Look at                                      |
| ---------- | ----------------------- | ------------------------------------- | -------------------------------------------- |
| `#diverge` | What else could we do?  | 2, each given a different constraint  | The new ideas                                |
| `#rank`    | Which option is best?   | 2, given the same brief               | Ties — pairs with no consistent winner       |
| `#judge`   | Is this good?           | 3, given the same brief               | Reviewer disagreement                        |
| `#falsify` | How does this fail?     | 3, given the same brief               | Failures nothing would catch                 |

Each prompt ends with a "when this approach is the wrong one" section that
redirects to a sibling — e.g. `#judge` points to `#falsify` when you want
failure modes rather than a quality ruling.

### `#diverge <the problem>`

The only analysis prompt that gives each subagent a different brief: two
constraints chosen to push in different directions ("solve it so the obvious
approach is the one that fails", "solve it at a different layer"). Each
subagent develops one approach and describes its tradeoffs — it does not list
options.

Use it when the options already on the table feel like variations of one
idea. The report has a section listing any new approach that turned out to be
a restatement of an existing one; if all of them do, that means the current
options already cover the space.

### `#rank <list of choices>`

Compares three or four options. Two is treated as too few to compare, and the
prompt will not add options itself. Both subagents each run the full
comparison independently, and each pair is compared twice, in both orders.

- A pair whose winner changes depending on order is reported as a tie.
- A circular result (A beats B, B beats C, C beats A) is reported as having
  no winner.
- If both subagents produce the same order, the report says the ranking is
  reliable.

### `#judge <a thing>`

The agent first writes the rubric it will judge against: MUST include
(required properties), ACCEPTABLE alternative (correct approaches that differ
from the obvious one), MUST NOT (disqualifying problems). Three subagents
then each rule PASS, PARTIAL, or FAIL against that rubric and quote evidence.

The report shows the table of verdicts and highlights any disagreement. If
all three pass, the report says so. A subagent that errored is counted as
failed, not skipped.

### `#falsify <a thing>`

Name the thing and what it is supposed to do; if you only name the thing, the
agent infers the goal from the conversation. Three subagents each look for
ways it fails. Each finding must include a trigger (the input or state that
sets it off), an explanation of how the goal breaks, the severity, and a
quoted line of evidence. Findings without evidence are dropped.

- Failures nothing would catch automatically are reported first.
- The report includes "what held up": attacks that were tried and could not
  be made to work.
- It does not propose fixes unless asked.

## The generating prompts

### `#metaprompt <repeatable 1-shot task>`

Writes a new prompt template for a task you expect to repeat. Intended for
small tasks that take 1–2 prompts, not full features. Save the result to
`~/.config/jinn/prompts/` and it becomes a `#token`; press `gcr` in normal
mode to re-scan the directory without restarting.

### `#generate-persona <description>`

Writes a complete persona file from a description, e.g. "ruthless article
editor" or "pedantic code reviewer". The generated persona is written in
prose rather than bullets, because the model copies a persona's formatting
style in its responses. Save it to `~/.config/jinn/personas/`, then select it
with the persona picker (`<leader>se`) or edit it with `c` from the sidebar's
Persona section.

## The research prompt

### `#research <topic>`

The longest builtin workflow, and the only one that stops for approval before
doing the work:

1. **Orient.** 2–4 broad searches to map the topic.
2. **Interrogate.** At most 2 rounds of questions, batched into one message.
   It infers what it can from your request and asks only what it cannot.
   Anything still unknown is written into the brief as a stated assumption.
3. **Brief.** It proposes a research brief: objective, scope, source
   categories (it suggests which kinds of sources to use — papers,
   practitioner blogs, issue trackers, forums — and asks which to weight),
   and what the report will contain. The full research does not start until
   you approve the brief.
4. **Report.** Findings are grouped by source category, and each source is
   classified at the point of use. Where categories disagree, the report
   shows the disagreement rather than merging the findings into one
   conclusion.

The prompt does not write your output artifact (the post, the paper); it
produces the research data. Gaps count as findings: if no good source exists
for a claim, the report says so. Output is saved to `./research/` with
markdown footnote citations.

## The code workflow prompts

`#plan`, `#approve-plan`, `#goal`, and `#gap-analysis` implement the
plan→implement→verify workflows described in the README.

`#plan` and `#goal` both consult `.agents/RECORD.md` — the project's record
of current-state facts — before asking questions, and both produce a
**Decision Ledger**: a table with one row per question the dialogue settled,
containing your wording of the decision, the rejected alternatives, and
whether the choice was yours (`user`) or the agent's (`agent`). This table
matters because both prompts produce a document that is handed to an
implementer without the conversation — the ledger is the only record of what
you decided.

### `#plan <thing to make/do>`

Runs a Socratic dialogue about how to build the thing. The agent asks
numbered questions (lettered options where real alternatives exist, with a
recommendation while the question is open) and traces the code as it goes, so
its questions describe actual current behavior.

- It does not propose a plan while questions are still open.
- It will re-open a settled question only once, in one sentence naming the
  consequence.
- Saying "just do it" (or raising the same closed question twice) ends the
  dialogue, and it proceeds with what was decided.
- Nothing is written to disk during the dialogue.

The output is a short High-Level Plan: Problem, Solution, Phases, Acceptance
Criteria, and a table of test cases. Follow up with `#approve-plan` when it
looks right.

### `#approve-plan`

Converts the approved plan into a detailed implementation specification and
saves it to `.plans/<task>/plan.md` with the `save_plan` tool. The spec is
written to stand alone: an implementer with none of your conversation can
implement from it. It copies the Decision Ledger verbatim, then checks the
spec against the ledger — any decision in the spec that the dialogue never
settled is flagged or removed. It then builds the todo list from the plan's
phases, plus a final Verification phase containing each acceptance criterion,
and shows you the ledger one last time to confirm each row.

### `#goal <thing to accomplish>`

The same dialogue process as `#plan`, but about what to accomplish rather
than how. The output is a **Task Contract**: End State (checkable facts),
Invariants, Decision Rules for choices you delegate, Constraints, Traps,
Verification, and Acceptance Criteria. It contains no phases, no steps, and
no code — the agent decides the route itself.

Flow: question rounds → a short CONTRACT BRIEF (Problem, End State,
done-when) that you approve → the full contract. When you approve, the
conversation is discarded and the implementer receives only the contract, so
confirm the End State and Constraints sections say what you mean. Afterward,
a common pattern is to select the contract message, press `gcip` to
isolate-and-pin it, and tell the agent to begin.

### `#gap-analysis`

Read-only post-implementation check. It reads the plan/spec from `.plans/`,
then evaluates each Acceptance Criterion one by one, reporting met / unmet /
partial with evidence and a recommendation for anything unmet. If the plan
contained "Record Updates", it also compares `.agents/RECORD.md` against what
was actually implemented and reports entries that are missing, wrong, or
newly needed. It does not edit code.

## Choosing between them

The README compares the coding workflows in a full table. The short version:

- `#goal` — you can describe the end state but not the route. Fewer question
  rounds, then the agent works autonomously inside the contract.
- `#plan` + `#approve-plan` — you know how it should be built. More question
  rounds, but the implementation follows your decisions. Implement with the
  `simple-task-loop` skill (verifies and commits each phase) or
  `micro-task-loop` (one commit at the end); name the skill in the same
  message.
- `#gap-analysis` — optional last step for either workflow: an
  acceptance-criteria audit report.

The analysis prompts can be used at any point: `#diverge` before planning,
`#rank` when choosing between approaches, `#judge` on a plan or a diff,
`#falsify` before merging.
