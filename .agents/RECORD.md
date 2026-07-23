# The Record

A curated list of factual, scoped statements asserting the application's **current** state. Authoritative for the present, never the future.

The planner consults this file before proposing a plan. If a feature **contradicts** an entry here, the contradiction is surfaced before the plan proceeds. If a feature **establishes a new high-level fact**, a verbatim entry is proposed for human approval as part of the plan.

## Format Rules

- **Factual.** Assert how things are _now_. Never future intent ("we will...", "should..."). Each entry is the current state of the application.
- **Scoped.** Name what each entry applies to — repo, app, frontend, or a named subsystem. An unscoped fact (e.g. "uses Fossil") is ambiguous: is that the repo, or the app's supported VCS list? Always disambiguate.
- **High-level.** One-liners (a few sentences at most). Capture decisions and facts a planner needs, not implementation minutiae.

## Templates

| Pattern     | Form                                                             | Example                                                                                 |
| ----------- | ---------------------------------------------------------------- | --------------------------------------------------------------------------------------- |
| State       | `[Scope] currently [does X / is Y].`                             | "The TUI's first screen at startup is the chat screen."                                 |
| Persistence | `[Scope] persists [what] to [where].`                            | "Sessions persist to SQLite."                                                           |
| Flow        | `[Input/event] is handled by [actor/subsystem], which [action].` | "File edits route through the `edit` tool, which validates `LINE#HASH` anchors."        |
| Boundary    | `[Scope] is bounded by [constraint].`                            | "Project discovery walks ancestors until a VCS root or `$HOME`, whichever comes first." |

## Absence

A missing record, or an un-recorded area, simply means the list has no entry there yet. Absence is not a constraint — it is an open question, and a feature that fills a gap may establish the first entry for that area (proposed for human approval as part of the plan).

## Editing

Entries are added or amended only with human approval, via a "Record Updates" section surfaced in the approved plan. The gap-analysis step checks whether implementation actually changed a recorded fact and flags any needed update.

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

- The `bash` tool runs commands through `bash` (not `sh`, `fish`, or `dash`).
- The `bash` tool has a streaming output threshold that truncates accumulated output to prevent unbounded memory growth between timer ticks.
- The `bash` tool accepts an optional `max_duration_secs` argument that overrides the default timeout; the schema exposes `max_duration_secs`, not a raw `timeout`.
- When the `bash` tool or a built-in tool panics mid-execution, it publishes a failed-execution event rather than crashing the actor.

- The `read` tool truncates large files and reports the correct line numbers and the next offset to resume from in its notice.
- The `read` tool accepts `offset`/`limit` to page through files larger than its output cap.
- The `read` tool returns path-not-content for directories and matching line data for files.
- The `write` tool creates parent directories automatically and overwrites existing files.
- The `write` tool preserves BOM and CRLF line endings on round-trip; it handles filenames with spaces and Unicode.
- The `write` tool pins the tool result only on success — failed writes (bad JSON, dir creation failure, file write failure) produce no pin.
- The `read`/`write`/`edit` tools correctly round-trip null bytes, emoji, CJK, combining characters, backslashes, angle brackets, ampersands, and embedded quotes.
- The `grep` tool wraps ripgrep; it supports `--glob`, `--file-type`, and `--path`, and reports errors on invalid patterns.

- The `edit` engine rejects operations with unknown ops, empty edits, empty old-text, or missing files.
- The `edit` engine rejects edits whose `LINE#HASH` anchor no longer matches the file's current content ("stale anchor" rejection).
- The `edit` engine rejects pathological edits with long repeated-character runs (the "degenerate pathology" guard) while allowing legitimate repetition up to a bound.
- The `edit` engine validates that edits within one call come from the same prior `read`; conflicting or overlapping edits are rejected.
- After a successful edit, fresh anchors are returned for the changed region so the agent can chain edits without re-reading.

- The `skill` tool loads a skill's body by name from the discovered set and returns the body in the tool result; loading an already-loaded skill returns "already loaded" instead of reloading.
- The `skill` tool loads project-local skills from their discovered file path and refuses disabled or nonexistent skills.
- Skill scanning is triggered on session lifecycle events (created, cwd-changed, setup-completed) and on manual `ScanSkills` commands.
- A project skill overrides a global skill with the same name; the discovery walk collects ancestors least-local-first so most-local-wins is a later-overwrites-earlier pass.
- Skill scanning discovers an ancestor project skill from a nested cwd, and re-scanning the cwd clears previously discovered skills first.

- Context assembly builds the system prompt in priority order: skills block, pinned system entries, environment context, tool context.
- A forced system-prompt override replaces all generated system parts but still includes pinned system entries from history.
- Pin splitting separates entries into TOP pins, BOTTOM pins, and working history based on pin position.
- The `context_files_scan_actor` walks the bounded ancestor chain, reads the first existing candidate (AGENTS.md / CLAUDE.md) per dir, and writes results into the session's discovered set.
- `#name` prompt-template tokens in user text expand to the template body; `@path` tokens resolve to `file://` URIs against cwd/home and are consumed in a second expansion pass.

- Compaction is gated by a context-size threshold: it skips when below, triggers when at or above, and uses a fallback context length when the model isn't in the cache.
- Compaction preserves pinned entries; the cut index walks backwards from a reserve and advances past complete tool loops to a valid opener.
- The compaction gate is re-evaluated on subsequent events and prevents double-compaction after the first; `threshold=0` always triggers and `threshold=1` requires the full context.
- The compaction worker is per-session: clearing/compacting session A does not affect session B.
- The compaction gate splits on `provider/model` format and uses the session's model for the context-length lookup.

- History workers implement a `HistoryWorker` trait and are spawned via `actor_wiring.rs`; adding a new strategy means adding a worker file and wiring it.
- Auto-prune strategies exclude stale/redundant entries from LLM context; strategies include `anchor_shield`, `anchored_assistant`, `broken_edit`, `consecutive_reads`, `double_edit`, `edit_read`, `read_edit`, `min_age`, `regex`, `todo_prune`, `tool_age_window`, `trivial_assistant`.
- Auto-prune skips entries that are already excluded/forced (no duplicate mutations), and a user force-include overrides a worker force-exclude.
- Auto-prune respects a minimum entry age: entries at or below the age boundary are protected from pruning.
- Auto-steer (`todo_steer`) injects a steering `User` entry at the tail only when `todo_*` tool calls exceed a threshold, with a per-session pending guard.
- Auto-steer is keyed per-session, so a pending steer in one session does not suppress another session, and it clears once the pending id appears in history.

- There is a per-session steering buffer for mid-turn message injection; drained steering entries become normal User entries with the default context override and are never pinned.
- The queue actor drains the steering buffer before context assembly on both user-message dispatch and dispatch-resume.

- Archiving the last active session creates a new one; archiving an empty session removes and archives it; archiving the active session switches to the next one.
- An empty session that was never interacted with is not persisted on archive.
- A replacement session seeded on archive inherits reasoning effort from the global default.
- Entry kinds round-trip through serialization; attachments are allowed on vision-capable or unknown models, and image-to-text-only models are blocked with an error entry.
- Model selection supports alloy (multi-provider) configs that round-trip through serde; `as_single` returns `None` for an alloy and the string for a single model.

- Chat entry selection applies an accumulated-exclude guard that only takes effect after a threshold, with per-entry forced include/exclude tracked separately.
- The chat input popup narrows rows by typed prefix and renders directory entries with trailing slashes, plus empty/loading states.

- A stall watchdog detects sessions stuck in sending, mid-tool-batch stalls, and streaming sessions with no history change, and publishes a cancel after the budget is exhausted.
- The watchdog resets its stall counter at turn boundaries (not on activity jitter) and resets the budget when provider activity resumes; retries are suppressed within a backoff window.
- An idle session is never scanned by the watchdog, and an active streaming session is never flagged.
