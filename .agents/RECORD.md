# The Record

A curated list of factual, scoped statements asserting the application's **current** state. Authoritative for the present, never the future.

The planner consults this file before proposing a plan. If a feature **contradicts** an entry here, the contradiction is surfaced before the plan proceeds. If a feature **establishes a new high-level fact**, a verbatim entry is proposed for human approval as part of the plan.

## Format Rules

- **Factual.** Assert how things are _now_. Never future intent ("we will...", "should..."). Each entry is the current state of the application.
- **Scoped.** Name what each entry applies to — repo, app, frontend, or a named subsystem. An unscoped fact (e.g. "uses Fossil") is ambiguous: is that the repo, or the app's supported VCS list? Always disambiguate.
- **High-level.** One-liners (a few sentences at most). Capture decisions and facts a planner needs, not implementation minutiae.
- **Single tag.** Each entry carries exactly one subsystem tag as a `(tag)` prefix: `- (tools) The bash tool runs...`. One entry, one tag — this keeps tag usage a meaningful coverage metric (a tag growing large signals over-specification or a tag that should split). If you cannot decide between two tags for an entry, that is a signal to **re-evaluate the entry itself**, not to assign both. Use `(tag)` rather than `[tag]` to avoid colliding with markdown task-list (checkbox) syntax.

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

Entries are added or amended **only with human approval**.

---

<!-- Add entries below. Keep them scoped, factual, and high-level. -->

- (arch) A component/actor system built on `kameo` runs domain logic asynchronously, communicating via command routing and event broadcast.
- (arch) The `IntentHandler` mutates `AppState` directly and returns commands; it never touches external services or emits events.
- (arch) User input flows through a `Keymap` that produces an `Intent`; the `IntentHandler` handles intents synchronously as a single match block.
- (arch) `AppState` is the shared state; the frontend writes user input, domain actors write their owned fields, and the TUI renderer reads it on each tick.
- (chat) The queue actor drains the steering buffer before context assembly on both user-message dispatch and dispatch-resume.
- (compaction) Compaction is gated by a context-size threshold: it skips when below, triggers when at or above, and uses a fallback context length when the model isn't in the cache.
- (compaction) Compaction preserves pinned entries; the cut index walks backwards from a reserve and advances past complete tool loops to a valid opener.
- (compaction) The compaction gate is re-evaluated on subsequent events and prevents double-compaction after the first; `threshold=0` always triggers and `threshold=1` requires the full context.
- (compaction) The compaction gate splits on `provider/model` format and uses the session's model for the context-length lookup.
- (compaction) The compaction worker is per-session: clearing/compacting session A does not affect session B.
- (compaction) When working history exceeds the session token budget, entries are trimmed newest-to-oldest (pinned entries preserved) and a compaction prompt is injected.
- (context) A forced system-prompt override replaces all generated system parts but still includes pinned system entries from history.
- (context) Context assembly builds the system prompt in priority order: skills block, pinned system entries, environment context, tool context.
- (context) Pin splitting separates entries into TOP pins, BOTTOM pins, and working history based on pin position.
- (context) The `context_files_scan_actor` walks the bounded ancestor chain, reads the first existing candidate (AGENTS.md / CLAUDE.md) per dir, and writes results into the session's discovered set.
- (context) `#name` prompt-template tokens in user text expand to the template body; `@path` tokens resolve to `file://` URIs against cwd/home and are consumed in a second expansion pass.
- (dashboard) The dashboard tab tracks actor lifecycle (starting/running/dead) and browser-binary detection (Chrome vs bundled) for the web-fetch feature.
- (discovery) Project discovery walks ancestors from the session cwd up to either a VCS root or `$HOME`, whichever comes first; `$HOME` is exclusive.
- (discovery) VCS roots are detected by marker files (`.git`, `.hg`, `.fslckout`, `.fossil`, `.jj`), not by shelling out to a VCS CLI.
- (discovery) A discovery coordinator orchestrates project, browser-binary, file-listing, and skills scans across ancestor dirs; a notifier surfaces settled results to the session.
- (history) Auto-prune respects a minimum entry age: entries at or below the age boundary are protected from pruning.
- (history) Auto-prune skips entries that are already excluded/forced (no duplicate mutations), and a user force-include overrides a worker force-exclude.
- (history) Auto-prune strategies exclude stale/redundant entries from LLM context; strategies include `anchor_shield`, `anchored_assistant`, `broken_edit`, `consecutive_reads`, `double_edit`, `edit_read`, `read_edit`, `min_age`, `regex`, `todo_prune`, `tool_age_window`, `trivial_assistant`.
- (history) Auto-steer (`todo_steer`) injects a steering `User` entry at the tail only when `todo_*` tool calls exceed a threshold, with a per-session pending guard.
- (history) Auto-steer is keyed per-session, so a pending steer in one session does not suppress another session, and it clears once the pending id appears in history.
- (history) History workers implement a `HistoryWorker` trait and are spawned via `actor_wiring.rs`; adding a new strategy means adding a worker file and wiring it.
- (history) There is a per-session steering buffer for mid-turn message injection; drained steering entries become normal User entries with the default context override and are never pinned.
- (identity) **This repository** uses Fossil for version control (the app supports git/hg/jj/fossil via marker detection).
- (identity) **jinn** is a terminal-based agent harness written in Rust (edition 2024).
- (identity) Four personas ship by default: `coding-assistant`, `general`, `brainstorm`, and `learning-tutor`.
- (identity) The TUI is the default entrypoint; the Discord and headless frontends are alternative invocation modes.
- (identity) The application ships three frontends: a TUI (default), a Discord gateway, and a debug-only headless mode.
- (keybinds) Bare letters in pickers route to the filter input rather than triggering actions: `a` in the project picker types into the filter, and `d` removes the highlighted entry (not a filter character).
- (keybinds) In the skill scope, `PgUp` pages the picker list, not the preview, so list paging and preview scrolling are separate bindings.
- (keybinds) In the skill scope, `Ctrl+L` loads the highlighted skill into context as a pinned ToolResult paired with a synthetic ToolCall (the same on-disk shape the `skill` tool produces), auto-enabling a disabled skill first; the picker stays open so several skills can be loaded in one visit.
- (keybinds) In the skill scope, `Tab` cannot disable a skill already loaded into context — disabling would imply an unload that does not happen (the body stays pinned until it is unpinned and pruned). `Tab` is a no-op for a loaded skill.
- (keybinds) Leader-chord keybinds resolve multi-key sequences: `<leader>se` opens the persona picker, `<leader>sr` opens the reasoning-effort picker, and `<leader>[p]` jumps to pinned intents — chords that don't complete (e.g. `[c` in input scope) don't resolve.
- (keybinds) Picker scopes bind `PgUp`/`PgDn` to page-up/page-down of the picker list; in the skill and task-list scopes these also scroll a preview pane (`Ctrl+D`/`Ctrl+U` for the skill preview).
- (keybinds) The `p` prefix group in the sidebar does not drop the normal-scope pin binding (group bindings are scope-local and don't shadow cross-scope bindings).
- (keybinds) `Alt+Q` in input scope toggles input mode; `Alt+S` focuses the sidebar sessions section from both input and normal scopes.
- (keybinds) `s` in the sidebar task-list section opens the task-list picker.
- (paths) Config lives at `~/.config/jinn` (providers, prompts, personas, themes, `jinn.toml`).
- (paths) Data lives at `~/.local/share/jinn` (`sessions.db`).
- (paths) State/logs live at `~/.local/state/jinn` (`jinn.log`), falling back to the data dir on platforms without a state dir.
- (persona) Personas are markdown templates loaded via the prompt-template system; the persona picker (`<leader>se`) switches the active session persona.
- (providers) LLM responses stream as a unified `StreamEvent` type, decoupled from any provider's native stream format.
- (providers) The provider crate supports three backends: Anthropic, Google, and OpenAI-compatible.
- (selection) Chat entry selection applies an accumulated-exclude guard that only takes effect after a threshold, with per-entry forced include/exclude tracked separately.
- (session) A replacement session seeded on archive inherits reasoning effort from the global default.
- (session) An empty session that was never interacted with is not persisted on archive.
- (session) Archiving the last active session creates a new one; archiving an empty session removes and archives it; archiving the active session switches to the next one.
- (session) Entry kinds round-trip through serialization; attachments are allowed on vision-capable or unknown models, and image-to-text-only models are blocked with an error entry.
- (session) Model selection supports alloy (multi-provider) configs that round-trip through serde; `as_single` returns `None` for an alloy and the string for a single model.
- (skills) A project skill overrides a global skill with the same name; the discovery walk collects ancestors least-local-first so most-local-wins is a later-overwrites-earlier pass.
- (skills) Agent skills are discovered from `~/.agents/skills/*/SKILL.md` and `.agents/skills/*/SKILL.md`; project skills override global skills (most-local-wins).
- (skills) Prompt templates are markdown files with `+++` TOML frontmatter; `#name` tokens in user text expand to a template body.
- (skills) Skill scanning discovers an ancestor project skill from a nested cwd, and re-scanning the cwd clears previously discovered skills first.
- (skills) Skill scanning is triggered on session lifecycle events (created, cwd-changed, setup-completed) and on manual `ScanSkills` commands.
- (skills) The `skill` tool loads a skill's body by name from the discovered set and returns the body in the tool result; loading an already-loaded skill returns "already loaded" instead of reloading.
- (skills) The `skill` tool loads project-local skills from their discovered file path and refuses disabled or nonexistent skills.
- (skills) Two skills ship by default: `phased-task-loop` and `simple-task-loop`.
- (storage) Sessions and chat history persist to a SQLite database (`sessions.db` under the data dir).
- (storage) User-editable TOML files (`providers.toml`, `jinn.toml`) are written through a comment-preserving `DocumentPatcher`, never via plain serialization.
- (storage) `jinn.toml` holds user preferences and is auto-created if missing.
- (storage) `state.toml` holds machine-managed runtime state (e.g. last-selected model) and is NOT auto-created.
- (theme) The TUI supports dynamic themes via TOML files in `~/.config/jinn/themes/*.toml`, supporting ANSI name, ANSI code, hex, and RGB color formats.
- (tokens) A token-count actor estimates per-entry token usage; these estimates drive context-assembly sizing and compaction thresholds.
- (tools) After a successful edit, fresh anchors are returned for the changed region so the agent can chain edits without re-reading.
- (tools) File edits, reads, and other built-in tool calls all funnel through a single `tools_actor` chokepoint.
- (tools) The `bash` tool accepts an optional `max_duration_secs` argument that overrides the default timeout; the schema exposes `max_duration_secs`, not a raw `timeout`.
- (tools) The `bash` tool has a streaming output threshold that truncates accumulated output to prevent unbounded memory growth between timer ticks.
- (tools) The `bash` tool runs commands through `bash` (not `sh`, `fish`, or `dash`).
- (tools) The `edit` engine rejects edits whose `LINE#HASH` anchor no longer matches the file's current content ("stale anchor" rejection).
- (tools) The `edit` tool patches files using `LINE#HASH` anchors; the agent copies anchors from a prior `read` rather than reproducing old text verbatim.
- (tools) The `grep` tool wraps ripgrep; it supports `--glob`, `--file-type`, and `--path`, and reports errors on invalid patterns.
- (tools) The `read` tool accepts `offset`/`limit` to page through files larger than its output cap.
- (tools) The `read` tool returns path-not-content for directories and matching line data for files.
- (tools) The `read` tool truncates large files and reports the correct line numbers and the next offset to resume from in its notice.
- (tools) The `read`/`write`/`edit` tools correctly round-trip null bytes, emoji, CJK, combining characters, backslashes, angle brackets, ampersands, and embedded quotes.
- (tools) The `write` tool creates parent directories automatically and overwrites existing files.
- (tools) The `write` tool pins the tool result only on success — failed writes (bad JSON, dir creation failure, file write failure) produce no pin.
- (tools) The `write` tool preserves BOM and CRLF line endings on round-trip; it handles filenames with spaces and Unicode.
- (tools) The agent's built-in file tools are `read`, `write`, `edit`, `bash`, `grep`, `save_plan`, `get_time`, `session_query`, and `skill`.
- (tools) When the `bash` tool or a built-in tool panics mid-execution, it publishes a failed-execution event rather than crashing the actor.
- (ui) A section is shown only when non-empty (Pins requires pinned ids, TaskList requires tasks); empty sections are hidden.
- (ui) Mouse drag creates a dragging selection state, `finalize` transitions dragging to active, and `cancel` returns to idle.
- (ui) Paste events are coalesced: empty chunks are harmless, multiple paste chunks within a window merge into one (preserving order), and coalescing stops at the first non-paste event.
- (ui) Popups scale with terminal size: larger terminals get taller popups, small terminals use ~75% height, and a minimum size is enforced (otherwise the popup reports too-small).
- (ui) Scope transitions are driven by keybinds that emit routing intents; leaving a scope pops back to the prior one (e.g. picker/skill/task-list scopes return to normal on `Esc`).
- (ui) Selection rects find the smallest matching region for a position and exclude the right and bottom edges; the focus position is clamped to bounds.
- (ui) Sidebar confirm/insert flows resolve per section: Sessions `i` resolves to confirm-insert, Sessions `Enter` to confirm, and Pins `Enter` resolves to sidebar-leave.
- (ui) Text selection in the chat is modeled as a state machine (idle → dragging → active); selection can be forward or backward and both extract the same text.
- (ui) The TUI tracks focus as a scope stack (`FocusScope`); keys resolve differently per scope, and the active scope determines which bindings are available (e.g. the dashboard scope has no chat-history or sidebar bindings).
- (ui) The chat input popup narrows rows by typed prefix and renders directory entries with trailing slashes, plus empty/loading states.
- (ui) The dashboard tab layout uses the full terminal width; the chat tab layout is the normal chat layout.
- (ui) The sidebar can enter an interactive resize mode (via `sidebar_resize`) to adjust its width.
- (ui) The sidebar has four sections — Persona, Pins, TaskList, Sessions — with cyclic navigation (Persona→Pins→TaskList→Sessions and back).
- (ui) The sidebar restores history position when leaving Pins, and the Sessions section is anchored to the bottom of the sidebar.
- (ui) The chat-input autocomplete popups (`#` prompts, `/` commands, `@` attachments) anchor horizontally and vertically to the trigger token's wrapped visual line, floating directly above the cursor rather than the top of the input box.
- (watchdog) A stall watchdog detects sessions stuck in sending, mid-tool-batch stalls, and streaming sessions with no history change, and publishes a cancel after the budget is exhausted.
- (watchdog) An idle session is never scanned by the watchdog, and an active streaming session is never flagged.
- (watchdog) The watchdog resets its stall counter at turn boundaries (not on activity jitter) and resets the budget when provider activity resumes; retries are suppressed within a backoff window.
- (web) Web search runs via DuckDuckGo and web fetch supports concurrent requests; consulted sources are deduped and flushed as a Sources footer when the turn reaches a final assistant answer.
- (workflow) Commits use `just commit '<message>'`, which runs `fossil addremove --dotfiles` so dot-directories like `.agents/` are included.
- (workflow) The workspace is checked with `just check` (compile), `just test` (tests), and `just lint` (lints); all tests must pass before committing.
