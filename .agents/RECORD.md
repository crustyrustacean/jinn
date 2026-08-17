# The Record

A curated list of factual, scoped statements asserting the application's **current** state. Authoritative for the present, never the future.

The planner consults this file before proposing a plan. If a feature **contradicts** an entry here, the contradiction is surfaced before the plan proceeds. If a feature **establishes a new high-level fact**, a verbatim entry is proposed for human approval as part of the plan.

## Format Rules

- **Factual.** Assert how things are _now_. Never future intent ("we will...", "should..."). Each entry is the current state of the application.
- **Scoped.** Name what each entry applies to — repo, app, frontend, or a named subsystem. An unscoped fact (e.g. "uses Fossil") is ambiguous: is that the repo, or the app's supported VCS list? Always disambiguate.
- **High-level.** One-liners (a few sentences at most). Capture decisions and facts a planner needs, not implementation minutiae.
- **Single tag.** Each entry carries exactly one subsystem tag as a `(tag)` prefix: `- (tools) The bash tool runs...`. One entry, one tag — this keeps tag usage a meaningful coverage metric (a tag growing large signals over-specification or a tag that should split). If you cannot decide between two tags for an entry, that is a signal to **re-evaluate the entry itself**, not to assign both. Use `(tag)` rather than `[tag]` to avoid colliding with markdown task-list (checkbox) syntax.
- **Singular concept.** Each entry should be a single sentence and only concerned with a single concept. Prefer multiple entries versus combining many things into one.

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
- (attachments) `@path` image resolution degrades on missing-file or non-image outcomes (token stays literal, turn dispatches); only a recognizable image failing conversion hard-blocks. A user entry carrying attachments is blocked unless the active model is confirmed image-capable via models.dev — unknown models are blocked, not allowed.
- (attachments) `@path` tokens in user entries are colored by resolution outcome in the chat render: green when attached as an image, red when degraded (missing file or not an image).
- (context) `#name` prompt-template tokens in user text expand to the template body; `@path` tokens resolve to `file://` URIs against cwd/home **when the file is a readable image, otherwise the token is left as literal text**; both are consumed in a second expansion pass.
- (dashboard) The dashboard tab tracks actor lifecycle (starting/running/dead) and browser-binary detection (Chrome vs bundled) for the web-fetch feature.
- (dashboard) `frontend.dashboard` is owned by `DashboardActor`, fed exclusively by events; `DiscordStatusActor` republishes its gateway status as a `DiscordStatusUpdate` event rather than writing the dashboard directly.
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
- (identity) Jinn's multimodal scope is bounded to image input (vision) and text output; it has no image-generation pipeline and no art/editing tooling.
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
- (keybinds) The `y` yank-selected-entry binding copies the entry's raw content to the clipboard: tool results yield untruncated output without the tool-name prefix, tool calls yield the raw JSON arguments, and ANSI escapes are stripped.
- (keybinds) `Alt+Q` in input scope toggles input mode; `Alt+S` focuses the sidebar sessions section from both input and normal scopes.
- (keybinds) `s` in the sidebar task-list section opens the task-list picker.
- (mcp) jinn is an MCP client: one `McpActor` per (session × enabled server) owns a connection to an MCP server over **stdio** (child process, JSON-RPC over stdin/stdout), **local_http** (jinn spawns a managed child process and connects via `StreamableHTTP`), or **remote_http** (jinn connects to an already-running server with no process management).
- (mcp) MCP tools are namespaced `mcp__<server>__<tool>` and registered per-session via `RegisterTools { session_id: Some(_) }`.
- (mcp) MCP server enablement is per-session, persisted in `SessionCore`, off by default; enabling spawns the actor+process, disabling kills both.
- (mcp) MCP servers are configured in `jinn.toml` under `[[mcp_server]]`.
- (mcp) MCP server child processes have piped stderr captured to a bounded ring buffer owned by each `McpActor`; stderr never reaches jinn's terminal.
- (mcp) Per-session MCP server status is owned by `McpCoordinatorActor`, driven by `McpServerStatus` events; it is surfaced in the sidebar, not the dashboard.
- (mcp) `McpActor` republishes its captured stderr tail via `McpServerLog` on a debounce while Running; `McpCoordinatorActor` owns the per-session tails alongside status.
- (mcp) The MCP server picker (`<leader>sM`) is a multipane inspector: a server list with a preview pane that toggles (Ctrl-prefixed) between a live stderr-tail/status view and the server's tool list.
- (mcp) For local_http servers, jinn parses the bind address from the server's `url` host, allocates a free port via bind-and-release, and injects both into the server's args via `<ip>`/`<port>` replacement tokens; the `<port>` token is also expanded in the `url` itself. The global `mcp_bind_address` preference has been removed — the bind address is per-server in the `url`.
- (mcp) HTTP connect has no wall-clock timeout: a server stays `Starting` until the HTTP endpoint is reachable, and is marked `Dead` only when the child process exits (captured stdout/stderr explain why).
- (mcp) A `remote_http` server (transport = "remote_http") connects to an externally-managed HTTP server at the configured `url` with no process management; `command` is optional (unused for remote_http).
- (mcp) MCP connections are monitored post-connect: `McpActor` spawns a liveness watcher that polls `is_transport_closed()` and publishes `Dead` when the connection drops, working uniformly across stdio, local_http, and remote_http transports. local_http connections additionally run a child-exit watcher that reaps the child (preventing zombies) and tears down the transport on process death, since a half-open TCP socket does not trip `is_transport_closed()` on its own. No auto-restart — a dead connection surfaces in the sidebar/picker for the user to restart via the inspector.
- (mcp) The `restart_mcp_server` built-in tool lets the model restart a dead MCP server by name (or by stripping a `mcp__<server>__<tool>` namespace). It awaits the coordinator's restart result — which resolves when the new connection succeeds (`Running`) or fails (`ConnectFailed`), with a 60s `Timeout` if neither — before returning, so the model cannot retry a tool call mid-startup. On any failure (including timeout) the result instructs the model to stop and wait for the user.
- (tools) Actor-provided tools route by their registration `provider` prefix via the generic `ExecuteTool` command, not a hardcoded per-name match; `web-fetch`/`web-search` remain distinct provider keys.
- (paths) Config lives at `~/.config/jinn` (providers, prompts, personas, themes, `jinn.toml`).
- (paths) Data lives at `~/.local/share/jinn` (`sessions.db`).
- (paths) State/logs live at `~/.local/state/jinn` (`jinn.log`), falling back to the data dir on platforms without a state dir.
- (plugins) Plugins are WASM components hosted in-process by jinn itself (wasmtime, one store per plugin, task-supervised), speaking NDJSON over in-memory pipes; the wasm sandbox is the isolation boundary.
- (plugins) Plugin configuration lives in `jinn.toml` under `[[plugin]]`; plugins spawn at app start and activate only after a jinn restart.
- (plugins) A plugin coordinator actor validates and authorizes all inbound plugin messages and caches contributions into `AppState`; synchronous consumers (pickers, renderer, assembly) read only the cache, never the plugin.
- (plugins) Plugin filesystem and network access is granted per-plugin in the manifest (preopened path allowlist, `http` bool); every plugin additionally gets a writable scratch dir under the data dir.
- (plugins) The plugin wire contract is a hand-maintained JSON Schema kept in sync with the `jinn-plugin-api` types by a drift test; plugin SDKs are distributed via `jinn plugin sdk` (GitHub), not crates.io.
- (persona) Personas are markdown templates loaded via the prompt-template system; the persona picker (`<leader>se`) switches the active session persona.
- (providers) LLM responses stream as a unified `StreamEvent` type, decoupled from any provider's native stream format.
- (providers) The provider crate supports three backends: Anthropic, Google, and OpenAI-compatible.
- (providers) Model output is text-only: the `StreamEvent` pipeline and assistant chat entries carry no image variant.
- (providers) OpenRouter serves each model through multiple upstream **endpoints**, each identified by a routing **tag**; the per-model `GET /api/v1/models/<id>/endpoints` payload lists them with pricing and uptime.
- (providers) OpenRouter endpoint listings are cached **in memory, per model, for the application's lifetime** (not persisted to disk); the picker serves cached entries on open and re-fetches on-demand via `<c-r>` (`RefreshEndpoints`).
- (providers) A `--dump-requests <dir>` CLI flag writes one JSON file per provider generation send (main dispatch and compaction), capturing the full assembled request payload verbatim; off by default.
- (providers) Model metadata (`context_length`, `input_modalities`, `extra_body`) is set per model in `providers.toml` via `[[providers.<name>.model_info]]` tables nested under the provider's map-keyed block.
- (providers) `providers.toml` declares providers as map-keyed tables (`[providers.<name>]`); provider order in the file carries no meaning, and duplicate names are rejected by TOML parsing.
- (providers) Model metadata precedence is: per-model config > provider-block config > API-discovered cache > models.dev.
- (providers) `providers.toml` is hand-authored only; discovered models are never written into it.
- (selection) Chat entry selection applies an accumulated-exclude guard that only takes effect after a threshold, with per-entry forced include/exclude tracked separately.
- (session) A replacement session seeded on archive inherits reasoning effort from the global default.
- (session) An empty session that was never interacted with is not persisted on archive.
- (session) Archiving the last active session creates a new one; archiving an empty session removes and archives it; archiving the active session switches to the next one.
- (session) Entry kinds round-trip through serialization; image attachments are allowed only on models confirmed image-capable via models.dev — text-only and unknown models are blocked with an error entry.
- (session) Model selection supports alloy (multi-provider) configs that round-trip through serde; `as_single` returns `None` for an alloy and the string for a single model.
- (session) A session can pin one OpenRouter endpoint on its profile; when pinned and the model is served via the OpenRouter backend, dispatch forces that endpoint with `provider.order=[tag]` and `allow_fallbacks:false` for prefix-cache affinity.
- (session) An endpoint pin applies only to a Single (non-alloy) model served via the OpenRouter backend; it is ignored for alloys and all other backends.
- (skills) A project skill overrides a global skill with the same name; the discovery walk collects ancestors least-local-first so most-local-wins is a later-overwrites-earlier pass.
- (skills) Agent skills are discovered from `~/.agents/skills/*/SKILL.md` and `.agents/skills/*/SKILL.md`; project skills override global skills (most-local-wins).
- (skills) Prompt templates are markdown files with `+++` TOML frontmatter; `#name` tokens in user text expand to a template body.
- (skills) Skill scanning discovers an ancestor project skill from a nested cwd, and re-scanning the cwd clears previously discovered skills first.
- (skills) Skill scanning is triggered on session lifecycle events (created, cwd-changed, setup-completed) and on manual `ScanSkills` commands.
- (skills) Skill supplementals live in spec-standard scripts/, references/, and assets/ directories beside SKILL.md; the `<available_skills>` block and skill tool result each surface the skill's absolute base_dir so the agent can resolve relative links in a skill body without derivation.
- (skills) The `skill` tool loads a skill's body by name from the discovered set and returns the body in the tool result; loading an already-loaded skill returns "already loaded" instead of reloading.
- (skills) The `skill` tool loads project-local skills from their discovered file path and refuses disabled or nonexistent skills.
- (skills) Two skills ship by default: `phased-task-loop` and `simple-task-loop`.
- (skills) The skill picker's rendered markdown previews are cached in an app-lifetime cache keyed by (body content hash, width); skill rescans and session lifecycle events never invalidate it.
- (skills) The skill preview cache is cleared only on theme change (via FrontendCaches::invalidate_all); its memory usage is unbounded by design.
- (storage) Sessions and chat history persist to a SQLite database (`sessions.db` under the data dir).
- (storage) User-editable TOML files (`providers.toml`, `jinn.toml`) are written through a comment-preserving `DocumentPatcher`, never via plain serialization.
- (storage) `jinn.toml` holds user preferences and is auto-created if missing.
- (storage) Startup fail-fast: a malformed providers.toml or jinn.toml aborts launch before actor wiring with a stderr report naming the path and TOML detail; recovery via jinn config subcommands stays unguarded.
- (storage) `state.toml` holds machine-managed runtime state (e.g. last-selected model) and is NOT auto-created.
- (storage) Schema migrations run atomically in a single transaction; a crash or interrupt mid-migration rolls back to the last-applied version, leaving no partial schema.
- (theme) Theme discovery flows through the built-in "themes" plugin: it scans `~/.config/jinn/themes/*.toml` (ANSI name, ANSI code, hex, RGB formats) and contributes full theme definitions over the plugin wire; the theme picker reads the contribution cache, not disk.
- (tokens) A token-count actor estimates per-entry token usage; these estimates drive context-assembly sizing and compaction thresholds.
- (tokens) The session token ledger stores the pre-send local estimate (`tokens_sent`) alongside provider-reported `prompt_tokens` and `cached_tokens` per request; the estimate is never overwritten.
- (tokens) The status-bar `↑sent` count uses the provider-reported `prompt_tokens` when a turn completed with usage, falling back to the estimate for turns without usage.
- (tokens) The status bar shows a cache-hit percentage (`⬢` glyph, leftmost) for OpenAI-compatible providers when cached prompt tokens are reported, computed over turns that reported usage.
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
- (tools) The agent's built-in file tools are `read`, `write`, `edit`, `bash`, `grep`, `save_plan`, `get_time`, `session_query`, `restart_mcp_server`, and `skill`.
- (tools) Programmatic image files a tool writes (`.png`, `.svg`, charts) are artifacts of the existing file-tool pipeline, not a model image-output capability.
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
- (ui) The `@path` autocomplete popup window-scrolls its entries to keep the selected entry in view when the filtered list exceeds the popup height, matching the `#`/`/` popups.
- (ui) The dashboard tab layout uses the full terminal width; the chat tab layout is the normal chat layout.
- (ui) The sidebar can enter an interactive resize mode (via `sidebar_resize`) to adjust its width.
- (ui) The sidebar has five sections — Persona, Pins, TaskList, McpServers, Sessions — with cyclic navigation.
- (ui) The sidebar restores history position when leaving Pins, and the Sessions section is anchored to the bottom of the sidebar.
- (ui) The chat-input autocomplete popups (`#` prompts, `/` commands, `@` attachments) anchor horizontally and vertically to the trigger token's wrapped visual line, floating directly above the cursor rather than the top of the input box.
- (watchdog) A stall watchdog detects sessions stuck in sending, mid-tool-batch stalls, and streaming sessions with no history change, and publishes a cancel after the budget is exhausted.
- (watchdog) An idle session is never scanned by the watchdog, and an active streaming session is never flagged.
- (watchdog) The watchdog resets its stall counter at turn boundaries (not on activity jitter) and resets the budget when provider activity resumes; retries are suppressed within a backoff window.
- (web) Web search runs via DuckDuckGo and web fetch supports concurrent requests; consulted sources are deduped and flushed as a Sources footer when the turn reaches a final assistant answer.
- (web) Browser-backed web tools (fetch + search) keep their Chromium process warm via a periodic heartbeat; a missed liveness probe force-evicts the handle so the next request lazily launches a fresh browser rather than hanging on a dead WebSocket.
- (workflow) Commits use `just commit '<message>'`, which runs `fossil addremove --dotfiles` so dot-directories like `.agents/` are included.
- (workflow) The workspace is checked with `just check` (compile), `just test` (tests), and `just lint` (lints); all tests must pass before committing.
