# Bots

A bot is a **session that references a parent without inheriting its
conversation**. The parent is an ordinary jinn session; the bot is a
second session that watches it. What crosses over is the _environment_ — the
creating session's cwd, project, profile (model, persona, tool/skill filters,
reasoning effort), and enabled MCP servers — and nothing else. The bot
starts with an empty history and builds its own.

That is the whole distinction from a subagent (see
`sessions-and-subagents.md`): a `spawn_subagent` subagent is a session the user asks for
once and which inherits the parent's working context, while a bot runs
on a condition, re-reads the parent each time, and reports back.

Use a bot for work that should re-run whenever a session finishes a
turn — a reviewer, a test-watcher, a nudge-when-it-stalled agent. Its two
bot-only tools are what make that loop possible: `conclude` records a
verdict the user reads, and `notify_parent` starts a turn in the parent when
the parent needs to act. Both are filtered out of every **non-bot**
session's tool list, so an ordinary session never sees them offered and
refused.

## Creating a bot

| Key          | Where                    | Action                                                       |
| ------------ | ------------------------ | ------------------------------------------------------------ |
| `b`          | Sessions sidebar section | New bot of the highlighted session, immediately active       |
| `B`          | Sessions sidebar section | New **standalone** bot (no parent) from the highlighted session's environment |
| `<leader>sb` | Normal mode              | Saved-bots picker — create one from `jinn.toml`              |

`b` lands you in a fresh bot in **prep mode**: nothing it holds can
dispatch yet, and anything you type is pinned into its context rather than
sent. You compose its standing instructions as pins, then turn prep mode off
(see below) to let it run.

A bot of a bot is not created — the create path refuses it, so
no lineage exists that the trigger rules have no answer for. `b` also refuses
when the highlighted session is busy, and only works from the Sessions
section.

`B` makes the same kind of session with **no parent at all**: the
highlighted session supplies only the environment (profile, working
directory, MCP enablement). A standalone bot's trigger is locked to
`manual` and its `notify_parent` tool is withheld — it keeps `conclude`,
so its report history still works — and its seed template starts empty.
It is the shape to use when the bot is a self-contained job rather than
something that watches a conversation.

In the saved-bots picker, `<enter>` attaches the entry's bot to the
active session (you stay where you are) and `<c-enter>` spawns it
standalone — no parent, same rules as `B` — and switches you to the new
bot's session. The entry's saved `prep_mode` is honored on either path.

## The properties popup

`P` opens the bot's properties over the highlighted session — bound
identically in the **Sessions** and the **Bots** sidebar sections; only
the highlighted row differs. The popup is a form of seven fields, in this
order:

| Field             | Values                       | What it governs                                                       |
| ----------------- | ---------------------------- | --------------------------------------------------------------------- |
| **Trigger**       | `manual`, `parent-completed` | When the bot re-runs on its own                                 |
| **Behavior**      | `reset`, `preserve`          | What a run _sees_ of the conversation (never whether it happens)      |
| **Prep mode**     | on / off                     | Whether the bot is still being composed — a gate, not a setting |
| **Tool set**      | `live`, `frozen`             | Whether new tools are admitted automatically or refused               |
| **Skill set**     | `live`, `frozen`             | Same, over skills                                                     |
| **Model**         | `inherit`, `fixed`           | Whether the model the session already holds belongs to this bot |
| **Seed template** | text                         | The instructions injected ahead of each run's prior report            |

`frozen` sets act as an allowlist, `live` sets act as a blocklist.

The **Model** row declares ownership; it does not choose a model. Every
bot holds a concrete model — it has to, in order to run — and this row
only says whether that model is the bot's own or the copy it inherited
when it was created. `inherit` (the default) writes no `model` key when the
bot is saved; `fixed` writes the model it holds, so recreating the
bot from its entry keeps it. Change the model itself from the status
bar's model picker, not from here.

### Popup keys

| Key               | Action                                                                 |
| ----------------- | ---------------------------------------------------------------------- |
| `j` / `k`         | Move the form cursor between the seven fields                           |
| `h` / `l`         | Pick a choice on the focused field (walk the row, clamped at its ends) |
| `i`               | Edit the seed template — only from the seed-template field             |
| `?`               | Toggle the help overlay                                                |
| `<c-s>`           | Save this bot to `jinn.toml` (stays open)                        |
| `<enter>`         | Apply all fields and close                                             |
| `<esc>` / `<c-c>` | Restore the values the popup opened with, and close                    |

The properties scope is **navigation-only**: it captures no typed input, so a
letter typed while the cursor is on a form field does nothing. Editing text
happens only in the seed-template editor. `<c-c>` is bound and behaves
identically to `<esc>` even though the popup's footer lists only `<esc>`.

### Seed-template editor keys

Opened with `i` from the seed-template field; it is its own text scope.

| Key                               | Action                                          |
| --------------------------------- | ----------------------------------------------- |
| any text / `<backspace>` / arrows | Edit the template                               |
| `<enter>`                         | Keep the edited template                        |
| `<esc>`                           | Restore the pre-editor template                 |
| `<c-c>`                           | Clear the template, or leave when already empty |

### Saving

`<c-s>` writes the bot as an `[[bot.entry]]` in `jinn.toml`
(comments and ordering on sibling entries survive). A save applies the
popup's pending values first, so what it writes is what the panel showed. The
session's **name** is the entry's key: saving under a name that already exists
**replaces that entry in place**, and a second `<c-s>` is required to confirm
the overwrite. A bot with no name cannot be saved.

## What a run does

Whichever way a run is started, it seeds a prompt from the **seed template**
and the bot's most recent `conclude` report, then dispatches it as a
fresh turn:

- The template's `<prior_report>` placeholder is replaced with the last
  report's body; on a first run (no report yet) it becomes the sentence
  "this is the first run, so there is no prior report".
- The template's `<parent_id>` placeholder is replaced with the parent
  session's id — a bot runs in its own session and has no other way to
  reach the transcript it reports on. A bot with no parent (a standalone
  bot) gets "unavailable" there.
- The seed renderer **only substitutes**: it appends nothing and removes
  nothing. An **empty template is valid** — the run dispatches whatever
  the composition produces (the message alone, for a message run).
- The default seed template for a parent bot is
  `"The parent session's id is <parent_id>. The previous run of this bot
reported: <prior_report>."` A standalone bot's default is empty.

### Trigger

- `manual` — runs only when the user re-runs it (`G` in the Sessions,
  Bots, or Library sidebar sections, or the properties popup's
  trigger being Manual simply means it never fires on its own).
- `parent_completed` — runs after a **parent turn completes successfully**.
  It fires **only on a turn that completed successfully**: an errored or
  cancelled parent turn does not fire it. A turn the automation itself
  started (a bot dispatching) does not wake that session's own
  bots, which is what stops two mutually-triggering bots from
  bouncing messages back and forth unattended.

A trigger fires **at most once per completed parent turn**, and the harness
imposes no cap on how often a bot runs overall — a bot that
fires on every successful parent turn is a per-turn job.

Note that the definition of a "turn" is one complete tool loop. So on a long
implementation task with thousands of tool calls, when the LLM finally finishes
and responds with it's terminal assistant response then that is the end of a
single turn.

### Behavior

The behavior decides what a run _sees_, never whether it happens:

- `reset` (default) — force-excludes every non-pinned entry, so the run sees
  the pins alone. The conversation is **not deleted**: the human still sees
  the full transcript, and the excluded entries can be un-hidden from context.
- `preserve` — leaves the conversation as it stands. A prior report is still
  folded into the seed (that is the bot's own record, not inherited
  conversation).

Both behaviors dispatch a message. The difference is the _history_, not
whether the prompt is sent.

## Prep mode is a gate

Prep mode (the `b` default) means the bot is still being composed.
While it is on:

- no manual re-run dispatches (`G` is refused),
- no trigger fires,
- a typed submission is **pinned into context instead of sent**.

A hand-written `[[bot.entry]]` that omits `prep_mode` gets the same
default, so it does not run until prep mode is explicitly turned off
(`prep_mode = false`). The rows above prep mode (trigger, behavior) are shown
dimmed while it is on, because they do not apply.

## Bot-only tools

`conclude` and `notify_parent` exist only inside a bot and are absent
from every other session's tool list:

- `conclude`: records a 1-line verdict in the bot's report log. This is
  used to carry information from previous runs through a reset and will be
  displayed in the sidebar for user informational purposes. Examples:
  - 8 of 10 tests passed
  - 2 of 3 judges agreed
  - Task list incomplete
  - Retried: 3 times
- `notify_parent`: send a message to the parent session.

## Messaging a bot (`tell_session`)

`tell_session` (available in every session) delivers a text message to any
session by id. On bots it is the dispatch mechanism:

- An **idle** bot starts a run immediately, seeded from its template
  plus the message.
- A **busy** bot queues the message and runs it when its current turn
  finishes. Nothing is cancelled; nothing races.
- Any other session receives it as a new user turn.

## The Bots sidebar section

Attached bots are displayed in the sidebar for the active session. Moving
the cursor over them allows opening the session with `<enter>` or `i` (insert
mode). They display the most recent report, or "no reports yet" if no
report/conclusion has been filed. Pressing the `s` key on a bot in the
sidebar will show its historical reports. `G` re-runs the highlighted bot
(canceling any in-flight turn); `R` resets its context in place — every
non-pinned entry is force-excluded and nothing is dispatched, so the run
that follows sees the pins alone.

An `⇉` icon next to an attend means it will activate automatically on assistant
turn end. A bot with a `⏸` icon is in "prep" mode and will not execute.

## The Library sidebar section

`L` in the Sessions section bookmarks the highlighted session into the
**Library** — a flat sidebar section of saved sessions, listed whether
loaded or archived (archived members render dimmed; `<enter>` loads and
switches to one, unarchiving as a side effect). Members are hidden from
the Sessions list while they are in the Library, and removing one (`L`
again, from inside the Library) puts it back.

The same bot keys work over library members: `G` re-runs, `R` resets the
bot's context without dispatching, `P` opens a member bot's properties,
and `c` resumes a loaded member. Only a bot **with a parent** is refused
at the `L` move — a shelved parent-completed bot would silently never
fire, since parent links are immutable. Plain sessions, forks, subagents,
and standalone bots may all enter.

## Saved bots (`[[bot.entry]]`)

A bot saved from the properties popup becomes one `[[bot.entry]]`
block in `jinn.toml`, restorable by name. The saved-bots picker
(`<leader>sb`) re-reads the document on every open, so a hand edit shows up
**without a restart** — this is the one `jinn.toml` surface that is live.

Creating from an entry reproduces its **configured** fields verbatim and
inherits the rest of the environment from the creating session — the same
environment a fresh `b` bot gets. The inheritance rule for every field:

> An **absent** field is inherited from the creating session; a **present**
> field overrides.

For every field, "present" means the literal key is there: an **empty
allow-list is meaningful** — `{ mode = "allow", names = [] }` says "this
bot may use no skills" and is distinct from omitting the field (inherit
the parent's). Do not strip empty arrays.

| Field              | Type                                                                     | When absent                                                      |
| ------------------ | ------------------------------------------------------------------------ | ---------------------------------------------------------------- |
| `name`             | string                                                                   | Required — the identity, and the key the save matches entries by |
| `behavior`         | `"reset"` \| `"preserve"`                                                | `reset`                                                          |
| `trigger`          | `"manual"` \| `"parent_completed"`                                       | `manual`                                                         |
| `prep_mode`        | bool                                                                     | `true` (still composing)                                         |
| `seed_template`    | string                                                                   | `"The parent session's id is <parent_id>. The previous run of this bot reported: <prior_report>."` |
| `model`            | `{ single = "..." }` or alloy                                            | Inherit the creating session's model — the panel's Model row reads `inherit` |
| `persona_name`     | string                                                                   | Inherit the creating session's persona                           |
| `tool_filter`      | `{ mode = "deny"\|"allow", names = [...] }`                              | Inherit the parent's tool filter                                 |
| `skill_filter`     | `{ mode = "deny"\|"allow", names = [...] }`                              | Inherit the parent's skill filter                                |
| `reasoning_effort` | `"max"`\|`"xhigh"`\|`"high"`\|`"medium"`\|`"low"`\|`"minimal"`\|`"none"` | Inherit the parent's effort                                      |
| `endpoint`         | `{ tag = "...", provider_name = "..." }`                                 | No OpenRouter endpoint pinned for this bot                 |
| `pins`             | `[{ role = "user"\|"assistant", text = "..." }]`                         | No standing instructions                                         |

Notes on the less obvious fields:

- `pins` are the bot's standing instructions, saved in history order —
  order is the point, because a pinned sequence is what `reset` leaves visible.
  A pin **requires** a `role`; a pin without one fails to parse, deliberately,
  so an agent result is never restored as a user instruction. Only `user` and
  `assistant` entries cross into the saved form; a pinned tool result does
  not (re-injecting a snapshot of a file would assert stale contents).
- `endpoint` records the OpenRouter routing slug a run was pinned to. Routing
  itself is resolved per model from `[[endpoint_defaults]]` in
  `providers.toml` at dispatch time, not from the entry (see
  `models-and-providers.md`), so this field is documentation of the pin rather
  than a routing override of its own.
- Model selection can be important for bots, so if you are making one on
  behalf of the user, ask if they want to use a specific model or if they want to
  inherit the session model.
- `model` is the panel's **Model** row, and the two stay in step: a `fixed`
  bot writes this key, an `inherit`ing bot writes nothing, and an
  entry that carries the key recreates as `fixed`. So a hand-written entry
  without a `model` key survives being saved from the popup unchanged, rather
  than acquiring a pinned model on that first save. Note the consequence of
  inheriting: the bot keeps whatever model it was created with, which is
  not re-read from the creating session on a later run.

### A complete example

The shipped default `jinn.toml` includes a worked entry you can copy. It
ships as `auto-nudge` and might not be present if the user deleted it.

```toml
# ###########################################################################
[bot]

# > "Nudges" a session to complete it's work if it stops mid-way. Relies
# > on the agent having a defined list of work and that it reports how far along
# > it is. This is NOT for evaluating the quality nor checking if the work was
# > actually implemented.
[[bot.entry]]
behavior = "reset"
name = "auto-nudge"
pins = [{ role = "user", text = """
You are in charge of keeping another agent on track. Please read the most recent message from the parent using the `session_fetch` tool and determine if it stopped prematurely. If it stopped prematurely, use the `notify_parent` tool to tell the agent to continue. The "continue" explanation should be brief but firm like "Continue with the <task>".

Examples of agents that stopped too early:
> I am on phase 3 of 5 <explanation of why it stopped>.

Examples of agents that stopped correctly:
> I've finished all phases of the plan.
> I had to stop because there is a problem with the current plan and I cannot continue because <reasons>.

It's important that you only tell the agent to continue if it stopped for something it _should not_ have stopped for. If it got blocked by something, then stopping is OK.

Regardless of the outcome, always use the `conclude` tool to record your conclusion:
- If you had to use `notify_parent` -> the "conclude" should be "Nudged"
- If the agent finished -> the "conclude" should be "Task complete"

You **do not** need to evaluate whether the agent actually did the work. Your job is simply to keep the agent moving if it says that it stopped before finishing. You don't need to check the work at all.""" }]
prep_mode = false
reasoning_effort = "high"
seed_template = "The parent session's id is <parent_id>. The previous run of this bot reported: <prior_report>."
trigger = "parent_completed"
tool_filter = { mode = "allow", names = ["conclude", "notify_parent", "session_fetch", "session_search"] }
skill_filter = { mode = "allow", names = [] }
```

What the example demonstrates, field by field:

- `trigger = "parent_completed"` + `behavior = "reset"` — it re-reads the
  parent after every successful turn, seeing only its own pins. The
  `session_fetch` tool in `tool_filter` is what reads the parent transcript.
- `prep_mode = false` — the one field a hand-written entry must state, or the
  bot will not run at all.
- `pins` — a single `user` pin holding the whole standing instruction. With
  `reset`, this pin is the only thing the run sees as prior context.
- `tool_filter` / `skill_filter` are both `allow` and both narrow: the
  bot may use exactly the two tools it exists to call plus the two it
  needs to read the parent, and **no skills at all** (`names = []` is the
  meaningful empty allow-list — the "may use no skills" case, not an absent
  filter).
- `reasoning_effort = "high"` — a bot is usually worth running on a
  cheaper, faster model than the parent's, which is what the missing `model`
  key below is about.

Note the `tool_filter` allow-list names `conclude` and `notify_parent`
explicitly. The harness keeps those two tools available to a bot
regardless of what a filter says, so the two entries are not strictly
required — but they document the intent, and a `deny` filter naming them
would really withhold them.

Also note the lack of a `model` field: the bot takes whatever model the
user has selected when it is created, and its Model row reads `inherit` — so
saving it back adds no `model` key, and the entry keeps inheriting.

## Picker and sidebar keys

Report-history picker (opened with `s` in the **Bots** section; see
`keybindings.md` for the section table). It is **read-only** — `<enter>` is
deliberately unbound.

| Key                                 | Action                                |
| ----------------------------------- | ------------------------------------- |
| `<esc>`                             | Close the report history              |
| `<c-c>`                             | Clear the filter, or close when empty |
| `<up>` / `<down>`                   | Move selection                        |
| `<pgup>` / `<pgdn>`                 | Page the list                         |
| any letter / `<backspace>` / arrows | Edit the filter                       |

Saved-bots picker (`<leader>sb` in Normal; `<enter>` creates the
highlighted bot on the active session):

| Key                                 | Action                                        |
| ----------------------------------- | --------------------------------------------- |
| `<enter>`                           | Attach this bot to the active session   |
| `<c-a>`                             | Attach and keep the picker open         |
| `<c-enter>`                         | Spawn this bot standalone, switch to it, and close |
| `<esc>`                             | Close without attaching                       |
| `<c-c>`                             | Clear the filter, or close when already empty |
| `<up>`/`<down>`/`<pgup>`/`<pgdn>`   | Move / page the list                          |
| any letter / `<backspace>` / arrows | Edit the filter                               |

## Cancellation

Cancelling a session recursively cancels its bot and subagent
descendants. The cascade **stops at forks**: a fork is an independent thread,
so a cancel does not reach a fork's own descendants.

The double-`Esc` prompt appears whenever the session, or anything a cancel
would reach beneath it, is still running — including when the session's own
turn has already finished and a bot is what remains. Confirming over
such a session stops the descendants below it and leaves the session itself
untouched.

## See also

- `keybindings.md` — the Bots sidebar section's key table
- `configuration.md` — `tool_filter` / `skill_filter` modes and the
  `[[bot.entry]]` snippet
- `context-management.md` — what pinning and forced-exclusion mean for a
  `reset` run
- `models-and-providers.md` — endpoint pinning and reasoning effort
