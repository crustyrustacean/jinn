# The Record

A curated list of factual, scoped statements asserting the application's
**current** state. Authoritative for the present, never the future.

The planner consults this file before proposing a plan. If a feature
**contradicts** an entry here, the contradiction is surfaced before the plan
proceeds. If a feature **establishes a new high-level fact**, a verbatim entry
is proposed for human approval as part of the plan.

## Format Rules

- **Factual.** Assert how things are *now*. Never future intent ("we will...",
  "should..."). Each entry is the current state of the application.
- **Scoped.** Name what each entry applies to — repo, app, frontend, or a named
  subsystem. An unscoped fact (e.g. "uses Fossil") is ambiguous: is that the
  repo, or the app's supported VCS list? Always disambiguate.
- **High-level.** One-liners (a few sentences at most). Capture decisions and
  facts a planner needs, not implementation minutiae.

## Templates

| Pattern | Form | Example |
| --- | --- | --- |
| State | `[Scope] currently [does X / is Y].` | "The TUI's first screen at startup is the chat screen." |
| Persistence | `[Scope] persists [what] to [where].` | "Sessions persist to SQLite." |
| Flow | `[Input/event] is handled by [actor/subsystem], which [action].` | "File edits route through the `edit` tool, which validates `LINE#HASH` anchors." |
| Boundary | `[Scope] is bounded by [constraint].` | "Project discovery walks ancestors until a VCS root or `$HOME`, whichever comes first." |

## Absence

A missing record, or an un-recorded area, simply means the list has no entry
there yet. Absence is not a constraint — it is an open question, and a feature
that fills a gap may establish the first entry for that area (proposed for
human approval as part of the plan).

## Editing

Entries are added or amended only with human approval, via a "Record Updates"
section surfaced in the approved plan. The gap-analysis step checks whether
implementation actually changed a recorded fact and flags any needed update.

---

<!-- Add entries below. Keep them scoped, factual, and high-level. -->

- **jinn** is a terminal-based agent harness written in Rust (edition 2024).
- The application ships three frontends: a TUI (default), a Discord gateway, and a debug-only headless mode.
- The TUI is the default entrypoint; the Discord and headless frontends are alternative invocation modes.

- User input flows through a `Keymap` that produces an `Intent`; the `IntentHandler` handles intents synchronously as a single match block.
- The `IntentHandler` mutates `AppState` directly and returns commands; it never touches external services or emits events.
- A component/actor system built on `kameo` runs domain logic asynchronously, communicating via command routing and event broadcast.
- `AppState` is the shared state; the frontend writes user input, domain actors write their owned fields, and the TUI renderer reads it on each tick.

- Sessions and chat history persist to a SQLite database (`sessions.db` under the data dir).
- `jinn.toml` holds user preferences and is auto-created if missing.
- `state.toml` holds machine-managed runtime state (e.g. last-selected model) and is NOT auto-created.

- Config lives at `~/.config/jinn` (providers, prompts, personas, themes, `jinn.toml`).
- Data lives at `~/.local/share/jinn` (`sessions.db`).
- State/logs live at `~/.local/state/jinn` (`jinn.log`), falling back to the data dir on platforms without a state dir.
- User-editable TOML files (`providers.toml`, `jinn.toml`) are written through a comment-preserving `DocumentPatcher`, never via plain serialization.

- The provider crate supports three backends: Anthropic, Google, and OpenAI-compatible.
- LLM responses stream as a unified `StreamEvent` type, decoupled from any provider's native stream format.

- The agent's built-in file tools are `read`, `write`, `edit`, `bash`, `grep`, `save_plan`, `get_time`, `session_query`, and `skill`.
- The `edit` tool patches files using `LINE#HASH` anchors; the agent copies anchors from a prior `read` rather than reproducing old text verbatim.
- File edits, reads, and other built-in tool calls all funnel through a single `tools_actor` chokepoint.

- The system prompt is assembled in priority order: skills block, pinned system entries, environment context, tool context.
- When working history exceeds the session token budget, entries are trimmed newest-to-oldest (pinned entries preserved) and a compaction prompt is injected.
- Agent skills are discovered from `~/.agents/skills/*/SKILL.md` and `.agents/skills/*/SKILL.md`; project skills override global skills (most-local-wins).

- Project discovery walks ancestors from the session cwd up to either a VCS root or `$HOME`, whichever comes first; `$HOME` is exclusive.
- VCS roots are detected by marker files (`.git`, `.hg`, `.fslckout`, `.fossil`, `.jj`), not by shelling out to a VCS CLI.

- Four personas ship by default: `coding-assistant`, `general`, `brainstorm`, and `learning-tutor`.
- Two skills ship by default: `phased-task-loop` and `simple-task-loop`.
- Prompt templates are markdown files with `+++` TOML frontmatter; `#name` tokens in user text expand to a template body.

- **This repository** uses Fossil for version control (the app supports git/hg/jj/fossil via marker detection).
- The workspace is checked with `just check` (compile), `just test` (tests), and `just lint` (lints); all tests must pass before committing.
- Commits use `just commit '<message>'`, which runs `fossil addremove --dotfiles` so dot-directories like `.agents/` are included.
