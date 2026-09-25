# The Record

A curated list of factual, scoped statements asserting the application's **current** state. Authoritative for the present, never the future.

The planner consults this file before proposing a plan. If a feature **contradicts** an entry here, the contradiction is surfaced before the plan proceeds. If a feature **establishes a new high-level fact**, a verbatim entry is proposed for human approval as part of the plan.

## Why This File Exists

A planner reads this file *instead of* reading the code, so an entry earns its place only by being **expensive to re-derive** — a decision, a boundary, a user-visible behavior, or a fact whose only copy is scattered across several files.

If a reader could recover the fact from one grep or one file read, it does not belong here. Most things do not belong here. The list is expected to be short, and adding an entry is a claim that the fact is not already obvious from the code.

Deleting an entry is always safe; a planner that needs it will find it. A record that grows to mirror the codebase costs every future planner and goes stale within weeks.

## Format Rules

- **Factual.** Assert how things are _now_. Never future intent ("we will...", "should..."). Each entry is the current state of the application.
- **Durable.** Every entry must survive a routine change. Apply the **rename test**: if the codebase renamed this thing tomorrow — a config key, a crate, a type, an actor, an event, a schema version — would the entry be false? If yes, it is not a fact about the application; it is a fact about today's source tree. Delete it.
- **Scoped.** Name what each entry applies to — repo, app, frontend, or a named subsystem. An unscoped fact (e.g. "uses Fossil") is ambiguous: is that the repo, or the app's supported VCS list? Always disambiguate.
- **High-level.** One-liners (a few sentences at most). Capture decisions and facts a planner needs, not implementation minutiae.
- **Single tag.** Each entry carries exactly one subsystem tag as a `(tag)` prefix: `- (tools) The bash tool runs...`. One entry, one tag — this keeps tag usage a meaningful coverage metric (a tag growing large signals over-specification or a tag that should split). If you cannot decide between two tags for an entry, that is a signal to **re-evaluate the entry itself**, not to assign both. Use `(tag)` rather than `[tag]` to avoid colliding with markdown task-list (checkbox) syntax.
- **Singular concept.** Each entry should be a single sentence and only concerned with a single concept. Prefer multiple entries versus combining many things into one.

## Templates

| Pattern     | Form                                                             | Example                                                                                     |
| ----------- | ---------------------------------------------------------------- | ------------------------------------------------------------------------------------------- |
| State       | `[Scope] currently [does X / is Y].`                             | "The TUI's first screen at startup is the chat screen."                                     |
| Persistence | `[Scope] persists [what] to [where].`                            | "Sessions persist to SQLite."                                                               |
| Flow        | `[Input/event] is handled by [actor/subsystem], which [action].` | "File edits route through the `edit` tool, which requires a unique match or `replace_all`." |
| Boundary    | `[Scope] is bounded by [constraint].`                            | "Project discovery walks ancestors until a VCS root or `$HOME`, whichever comes first."     |

## Absence

A missing record, or an un-recorded area, simply means the list has no entry there yet. Absence is not a constraint — it is an open question, and a feature that fills a gap may establish the first entry for that area (proposed for human approval as part of the plan).

## Editing

Entries are added or amended **only with human approval**.

---

- (context) Outgoing context assembly converts history entries to messages directly; a final tripwire validator drops any invalid tool loop with a tracing warning instead of sending invalid sequencing.
- (arch) All actors run on the trouper runtime, and schema-id-tagged messages route through BusService on trouper topics without bridge relays.
- (arch) Slices may import jinn-domain and foundation vocabulary; kernel code may consume kernel-adjacent vocabulary and lib-only slice behavior only where the dependency graph remains acyclic.
- (slices) Slice integration uses each slice's activation function from composition, with activation owning the slice's actors, cells, routes, and views.
- (slices) Slice activation crates and their paired `-msg` contract crates live under `crates/slices/`; shared and kernel-adjacent crates live under `crates/`.
- (slices) Slices read their `jinn.toml` section through read-only typed or dynamic config-section views; defaults are supplied by the slice.
- (arch) The `IntentHandler` mutates `AppState` directly and returns commands; it never touches external services or emits events.
- (arch) User input flows through a `Keymap` that produces an `Intent`; the `IntentHandler` handles intents synchronously as a single match block.
- (arch) `AppState` is the shared state; the frontend writes user input, domain actors write their owned fields, and the TUI renderer reads it on each tick.
- (context) jinn has no memory subsystem by decision: durable cross-session facts are carried by AGENTS.md/CLAUDE.md files, personas, and skills; cross-session recall is via the `session_search` and `session_fetch` tools; planning state is carried by pinned plan files.
- (compaction) Compaction is gated by a context-size threshold: it skips when below, triggers when at or above, and uses a fallback context length when the model isn't in the cache.
- (compaction) Compaction preserves pinned entries; the cut index walks backwards from a reserve and advances past complete tool loops to a valid opener.
- (compaction) The compaction gate is re-evaluated on subsequent events and prevents double-compaction after the first; `threshold=0` always triggers and `threshold=1` requires the full context.
- (compaction) The compaction gate splits on `provider/model` format and uses the session's model for the context-length lookup.
- (compaction) The compaction worker is per-session: clearing/compacting session A does not affect session B.
- (compaction) When working history exceeds the session token budget, entries are trimmed newest-to-oldest (pinned entries preserved) and the compaction prompt is sent as the explicit system prompt of the summary request.
- (context) A forced system-prompt override replaces all generated system sections; pinned history entries remain conversation messages regardless of override.
- (context) Context assembly builds the system prompt from dedicated per-section builders in fixed order: persona body, project context files, tool context, skills block, current date, working directory; empty sections are omitted.
- (context) The assembled system prompt travels as an explicit field through dispatch and provider requests; the message array carries only chat history, and provider request builders never extract content from it.
- (context) System-kind chat entries ride in LLM context as `[System]`-prefixed `User` messages in conversation order (when pinned or forced-include), never inside the system prompt.
- (context) Context assembly partitions history into top, bottom, and working outgoing groups while preserving assistant tool-call/tool-result loops as atomic units.
- (context) Task-list tool results include the next-step line.
- (context) Context assembly inserts only pin and working-history messages; there is no synthetic task-list snapshot injection.
- (attachments) `@path` image resolution degrades on missing-file or non-image outcomes (token stays literal, turn dispatches); only a recognizable image failing conversion hard-blocks.
- (attachments) A user entry carrying attachments is blocked unless the active model is confirmed image-capable via models.dev — unknown models are blocked, not allowed.
- (attachments) `@path` tokens in user entries are colored by resolution outcome in the chat render: green when attached as an image, red when degraded (missing file or not an image).
- (context) `#name` prompt-template tokens in user text expand to the template body; both token kinds are consumed in a second expansion pass.
- (context) `@path` tokens resolve to `file://` URIs against cwd/home when the file is a readable image; otherwise the token is left as literal text.
- (dashboard) The dashboard tab tracks actor lifecycle (starting/running/dead) per wired actor.
- (slices) Render slices live in per-slice typed cells behind the `Slices` facade; the owning actor, renderer, and intent router share typed handles to the registered cell.
- (keybinds) Feature keybinds are route rows carrying scope and key; keymap bindings are generated from registered rows at launch; dynamic intents and scope ids are data-carried, so an unregistered slice leaves no keymap, scope, or intent residue.
- (keybinds) The terminal overlay's keybinds are term-slice route rows binding the dynamic scopes term:view and term:control; no static terminal scope or terminal intent variants exist in the kernel.
- (keybinds) A slice key hook registers a per-scope catch-all returning a byte-carrying dynamic intent; key-hook and modal scopes are excluded from the GlobalToggle spread and the typing carve-out so capture mode stays hermetic.
- (keybinds) A slice can declare a dynamic scope modal through the route table; while a modal scope is on top, other slices' GlobalToggle rows and the typing carve-out do not pierce it — the scope's keys come from its own rows and hooks.
- (keybinds) Route row actions are `ActionFn` closures that receive an `ActionCtx` (`&mut AppState`, `&Slices`) lent by the intent handler at dispatch; state outside the slice's cells is reached only through the lent context.
- (discovery) VCS roots are detected by marker files (`.git`, `.hg`, `.fslckout`, `.fossil`, `.jj`), not by shelling out to a VCS CLI.
- (discovery) The skills, prompt-template, and context-file scans run per session inside the session-init slice's keyed discovery worker.
- (history) Auto-prune respects a minimum entry age: entries at or below the age boundary are protected from pruning.
- (history) Auto-prune skips entries that are already excluded/forced (no duplicate mutations), and a user force-include overrides a worker force-exclude.
- (history) Auto-prune strategies exclude stale/redundant entries from LLM context; the wired strategies are `anchored_assistant`, `broken_edit`, `consecutive_reads`, `double_edit`, `edit_read`, `read_edit`, `regex`, `todo_prune`, `tool_age_window`, `trivial_assistant`. `min_age` is not a strategy — it is a shared helper (`is_within_min_age`) giving individual workers an age floor.
- (history) History workers are limited to compaction and auto-prune strategies; no auto-steer worker exists.
- (history) There is a per-session steering buffer for mid-turn message injection; drained steering entries become normal User entries with the default context override and are never pinned.
- (history) Chat history is written only through the history editor API, which treats assistant tool-call/result loops as atomic chunks; entry-keyed operations expand to whole chunks with pin > user > worker precedence.
- (history) The `x` context toggle and pinning apply to a tool loop as a unit — a pinned or toggled member carries its whole loop.
- (identity) **This repository** uses Fossil for version control (the app supports git/hg/jj/fossil via marker detection).
- (identity) **jinn** is a terminal-based agent harness written in Rust (edition 2024).
- (keybinds) Bare letters in pickers route to the filter input.
- (keybinds) In the skill scope, `PgUp` pages the picker list, not the preview, so list paging and preview scrolling are separate bindings.
- (keybinds) In the skill scope, `Ctrl+L` loads the highlighted skill into context as a pinned ToolResult paired with a synthetic ToolCall (the same on-disk shape the `skill` tool produces); the picker stays open so several skills can be loaded in one visit.
- (keybinds) `Ctrl+L` in the skill scope auto-enables a disabled skill before loading it.
- (keybinds) In the skill scope, `Tab` cannot disable a skill already loaded into context — disabling would imply an unload that does not happen (the body stays pinned until it is unpinned and pruned). `Tab` is a no-op for a loaded skill.
- (keybinds) Leader-chord keybinds resolve multi-key sequences: `<leader>se` opens the persona picker, `<leader>sr` opens the reasoning-effort picker, and `<leader>[p]` jumps to pinned intents — chords that don't complete (e.g. `[c` in input scope) don't resolve.
- (keybinds) Picker scopes bind `PgUp`/`PgDn` to page-up/page-down of the picker list; in the skill and task-list scopes these also scroll a preview pane (`Ctrl+D`/`Ctrl+U` for the skill preview).
- (keybinds) The `p` prefix group in the sidebar does not drop the normal-scope pin binding (group bindings are scope-local and don't shadow cross-scope bindings).
- (keybinds) `Alt+Q` in input scope toggles input mode; `Alt+S` focuses the sidebar sessions section from both input and normal scopes.
- (keybinds) `s` in the sidebar task-list section opens the task-list picker.
- (mcp) The MCP server picker (`<leader>sM`) is a multipane inspector: a server list with a preview pane that toggles (Ctrl-prefixed) between a live stderr-tail/status view and the server's tool list.
- (mcp) For local_http servers, jinn parses the bind address from the server's `url` host, allocates a free port via bind-and-release, and injects both into the server's args via `<ip>`/`<port>` replacement tokens; the `<port>` token is also expanded in the `url` itself.
- (mcp) HTTP connect has no wall-clock timeout: a server stays `Starting` until the HTTP endpoint is reachable, and is marked `Dead` only when the child process exits (captured stdout/stderr explain why).
- (mcp) A `remote_http` server (transport = "remote_http") connects to an externally-managed HTTP server at the configured `url` with no process management; `command` is optional (unused for remote_http).
- (mcp) MCP connections are monitored post-connect: a liveness watcher polls for transport closure and publishes `Dead` when the connection drops, uniformly across stdio, local_http, and remote_http transports.
- (mcp) The `restart_mcp_server` built-in tool lets the model restart a dead MCP server by name (or by stripping a `mcp__<server>__<tool>` namespace).
- (mcp) MCP server entries accept a `headers` map; values support `${VAR}` env-var token expansion anywhere in the string, applied to both `local_http` and `remote_http` connections and ignored on stdio.
- (mcp) MCP header variables are resolved once at startup into the shared key store alongside provider keys; an unset or empty variable prevents connection with an error naming the variable, and header values are never logged or rendered.
- (mcp) MCP child processes (stdio and local_http) spawn terminal-isolated, like tool children.
- (paths) Config lives at `~/.config/jinn` (providers, prompts, personas, themes, `jinn.toml`).
- (paths) Data lives at `~/.local/share/jinn` (`sessions.db`).
- (paths) State/logs live at `~/.local/state/jinn` (`jinn.log`), falling back to the data dir on platforms without a state dir.
- (persona) Personas are markdown templates with TOML frontmatter; the persona picker (`<leader>se`) switches the active session persona.
- (providers) The provider crate supports three backends: Anthropic, Google, and OpenAI-compatible.
- (providers) Model output is text-only: the `StreamEvent` pipeline and assistant chat entries carry no image variant.
- (providers) A `--dump-requests <dir>` CLI flag writes one JSON file per provider generation send (main dispatch and compaction), capturing the full assembled request payload verbatim; off by default.
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
- (session) Lifecycle setup/teardown commands spawn terminal-isolated, like tool children.
- (skills) A project skill overrides a global skill with the same name; the discovery walk collects ancestors least-local-first so most-local-wins is a later-overwrites-earlier pass.
- (skills) Agent skills are discovered from `~/.agents/skills/*/SKILL.md` and `.agents/skills/*/SKILL.md`; project skills override global skills (most-local-wins).
- (skills) Prompt templates are markdown files with `+++` TOML frontmatter; `#name` tokens in user text expand to a template body.
- (skills) Skill scanning discovers an ancestor project skill from a nested cwd, and re-scanning the cwd clears previously discovered skills first.
- (skills) Skill scanning is triggered on session lifecycle events (created, cwd-changed, setup-completed) and on manual `ScanSkills` commands.
- (skills) Skill supplementals live in spec-standard scripts/, references/, and assets/ directories beside SKILL.md; the `<available_skills>` block and skill tool result each surface the skill's absolute base_dir so the agent can resolve relative links in a skill body without derivation.
- (skills) The `skill` tool loads a skill's body by name from the discovered set and returns the body in the tool result; loading an already-loaded skill returns "already loaded" instead of reloading.
- (skills) The `skill` tool loads project-local skills from their discovered file path and refuses disabled or nonexistent skills.
- (skills) The skill picker caches rendered previews, so reopening it and paging between skills is instant.
- (storage) Sessions and chat history persist to a SQLite database (`sessions.db` under the data dir).
- (storage) User-editable TOML files (`providers.toml`, `jinn.toml`) are written through a comment-preserving `DocumentPatcher`, never via plain serialization.
- (storage) `state.toml` holds machine-managed runtime state (e.g. last-selected model) and is NOT auto-created.
- (storage) Schema migrations run atomically in a single transaction; a crash or interrupt mid-migration rolls back to the last-applied version, leaving no partial schema.
- (theme) Themes are TOML files in `~/.config/jinn/themes/` (ANSI name, ANSI code, hex, RGB formats); the theme slice scans them into its cell at activation and the theme picker reads the cell, not disk.
- (tokens) The session token ledger stores the pre-send local estimate (`tokens_sent`) alongside provider-reported `prompt_tokens` and `cached_tokens` per request; the estimate is never overwritten.
- (tokens) The status bar shows a cache-hit percentage (`⬢` glyph, leftmost) for OpenAI-compatible providers when cached prompt tokens are reported, computed over turns that reported usage.
- (tokens) The status-bar cache-hit percentage is color-banded on its displayed value: >=95% uses theme.success, 90-94% uses theme.warning, below 90% uses theme.error_text.
- (ui) The minimap arrow shows per-entry o200k token counts summed over in-context entries above and below the cursor.
- (tools) After a successful edit, a numbered snippet of the changed region is returned so the agent can chain edits without re-reading.
- (tools) The `grep` tool wraps ripgrep; it supports `--glob`, `--file-type`, and `--path`, and reports errors on invalid patterns.
- (tools) Programmatic image files a tool writes (`.png`, `.svg`, charts) are artifacts of the existing file-tool pipeline, not a model image-output capability.
- (ui) Scope transitions are driven by keybinds that emit routing intents; leaving a scope pops back to the prior one (e.g. picker/skill/task-list scopes return to normal on `Esc`).
- (ui) The TUI tracks focus as a scope stack (`FocusScope`); keys resolve differently per scope, and the active scope determines which bindings are available (e.g. the dashboard scope has no chat-history or sidebar bindings).
- (ui) The chat input popup narrows rows by typed prefix and renders directory entries with trailing slashes, plus empty/loading states.
- (ui) The sidebar has five sections — Persona, Pins, TaskList, McpServers, Sessions — with cyclic navigation.
- (citations) Citable web sources detected in tool calls and results render as a Sources footer when a turn reaches a final assistant answer.
- (workflow) Commits use `just commit '<message>'`, which runs `fossil addremove --dotfiles` so dot-directories like `.agents/` are included.
- (workflow) The workspace is checked with `just check` (compile), `just test` (tests), and `just lint` (lints); all tests must pass before committing.
- (citations) The citations detector accepts `link` as a synonym for `url`, so Z.ai-shaped search results surface citations.
- (discord) Unauthorized slash-command use gets an ephemeral refusal; unauthorized plain messages are silently dropped.
- (subagents) Subagents are regular sessions spawned by the `task` tool: fresh history and an empty task list, linked to the parent, inheriting the parent's model, cwd, tools, skills, and MCP servers; they appear in the sidebar as children marked with a subagent symbol.
- (subagents) A subagent does not inherit the parent's task list — a copied list is context it did not ask for and cannot act on, so its whole assignment must arrive in the prompt. A subagent's own todo mutations never propagate to the parent's list.
- (subagents) The `task` tool blocks until the child session reaches Idle and forwards the child's last chat entry as its tool result; cancellations forward the cancel entry as a failure.
- (subagents) Subagent spawn stamps the `task` tool into the child's per-session `disabled_tools`, so it suppresses like any user-disabled tool and the tool picker reflects it; re-enabling it via the tool picker lets a subagent spawn subagents, and a session spawned by a re-enabled subagent starts suppressed again.
- (subagents) Forking strips the `task` tool from the fork's `disabled_tools`, so a fork of a subagent session always has the `task` tool enabled.
- (session) Sessions carry no automation flag; identity is a persisted origin enum (user, fork, subagent), and tree structure is linked via `parent_session`.
- (subagents) A spawned subagent's first dispatch waits for its discovery settle gate (project context files, skills, enabled MCP servers), bounded by an internal settle budget, so the first prompt includes MCP tools and project context; the message is sent regardless once the budget expires.
- (subagents) The sidebar's subagent marking reflects the session's origin, not the parent link; forks always get fork origin — even forks of subagent sessions.
- (subagents) A `task` tool-call entry carries an optional persisted link to the child session it spawned.
- (subagents) Enter on a selected `task` tool call activates its linked child session, loading it from the store if needed; archived children unarchive via the standard load path.
- (tools) The built-in `interactive_term`, `interactive_term_send`, and `interactive_term_kill` tools are the PTY interactive-terminal interface; each call blocks until screen output settles and returns the rendered screen.
- (tools) The `todo_set_list` tool accepts an empty `phases` array to clear the session's task list entirely.
- (tools) `interactive_term` PTY sessions persist across tool calls in a coordinator actor; the spawned program's lifetime is decoupled from tool calls.
- (tools) Agent input to an `interactive_term` session fails the tool call with only the wait notice while the user holds control — no screen is returned; that notice appears nowhere else — leaving control never messages the model.
- (tools) A tool call in flight when the user takes terminal control resolves with the wait notice instead of writing input; the user's keys reach the program through the same actor and the next agent call sees the user-driven screen.
- (tools) Each chat session has at most one interactive_term terminal: spawning again kills the previous one (reported in the result), and spawning without a chat session is rejected.
- (ui) The sidebar marks sessions with a live interactive_term terminal with a dedicated symbol.
- (session) The sidebar `X` key tears down the selected session and, on teardown success, archives the entire visible subtree (root and descendants) behind a press-again confirmation.
- (session) The stall-retry handler restarts only while the session is active and `stream_dispatched_at` is set; restarts cannot fire while a tool batch is in flight.
- (session) A session optionally carries a project association (a directory path) stamped only when the user picks a project at creation (the TUI projects UI or Discord /new, both backed by the curated `[[projects]]` list); it persists in the session metadata blob, is inherited by forks and subagents, and never follows cwd changes.
- (keybinds) `[` / `]` + `s` jumps the selection to the previous/next Sources (annotation) entry in chat history, clamping at the ends without wrapping.
- (prompts) Shipped prompts live in `res/prompts`, are embedded at compile time via the `BUNDLED` install catalogue, and `jinn install` seeds them to the user prompts dir, skipping files that already exist unless `--force`.
- (ui) The quake bar's session section shows both the currently-applied auto-prune token total and the pending accumulation total; the applied total derives from entry context-history at render time, excluding compaction and user-sourced excludes.
- (tokens) Per-entry token counts are a persisted, content-derived field on chat entries (entries.token_count column), computed once by the token count actor for entries lacking a count and saved by the regular session-snapshot persist path; no separate frontend token cache exists.
- (search) Sessions are searchable via an FTS5 index over persisted entry prose (user, assistant, tool_call, tool_result, system, error, compaction — never actor/thinking), keyed by (session_id, entry_id).
- (search) `session_search` passes queries to FTS5 MATCH unmodified and surfaces SQLite syntax errors verbatim; results are a flat bm25-ranked top-N with per-session rollup counts and no pagination.
- (search) A failed reindex leaves that session's dirty marker set (durable pending work), logs a warning with the session id, and never blocks the rest of the drain batch; a later drain retries it.
- (logs) -v controls only the verbosity of jinn\* crates; third-party crates always display WARN and ERROR (ERROR only at -q), and setting RUST_LOG overrides the automatic filter entirely.
- (logs) Trace colors are opt-in via `--trace-color`; default rendering is plain text (no ANSI escapes) in the trace file.
- (build) jinn's runtime/target link statically bundles SQLite via rusqlite's `bundled` feature (through daow's default `bundled-sqlite` feature); no system SQLite is used at runtime link time.
- (build) Building jinn from source on Windows requires no system SQLite installation.
- (build) Releases ship two cargo-binstall tarballs per tag: `x86_64-unknown-linux-gnu` and `x86_64-pc-windows-msvc` (both cross-built from Linux; the Windows artifact via cargo-xwin).
- (build) Release binaries are self-contained on both platforms: bundled SQLite in the target graph, no SQLite DLL/import-library requirement.
- (build) The Windows cross build (cargo-xwin) is wired entirely by env vars in the build-release-tarball recipe; no windows target config exists in .cargo/config.toml.
- (build) windows-gnu is not a supported release target; Windows release artifacts use the MSVC target.
- (pickers) The theme picker previews the highlighted theme live on cursor movement (invalidating theme caches per move), reverts to the snapshotted theme on ESC, and persists the choice only on confirm.
- (pickers) The tool picker toggles the highlighted tool with TAB (advancing to the next row), filters to tools available for the session's provider, seeds disabled state from the session profile (config seeds and subagent spawn stamps), and writes the disabled set back to the session only on confirm.
- (pickers) The session-lifecycle picker starts sessions with a scripted lifecycle from jinn.toml; entries whose setup command has $-parameters hand off to the arg-input popup before setup runs.
- (tools) The bash tool evaluates commands against the global and resolved project command policies before spawn; a match returns a failed tool result carrying the rule's message and the command never runs.
- (tools) Project command policy is resolved by cwd prefix match at tool-call time with the longest configured project path winning.
- (workflow) `just test` runs the workspace suite once with --no-fail-fast, tees the full cargo output to `target/test-output.log`, and prints a passed/failed summary including failing test names.
- (workflow) `just test-failures` extracts failing test names from `target/test-output.log` without re-running the suite.
- (workflow) `just test-one <filter>` runs workspace tests matching a name filter as the sanctioned iterate-on-failure path.
- (todo) The todo tool surface is `todo_set_list`, `todo_add_phase`, `todo_add_task`, `todo_get_phase`, `todo_get_task_list`, `todo_complete_task`, `todo_cancel_task`, `todo_postpone_task`, and `todo_postpone_to_phase`.
- (todo) The next-task indicator remains derived from list state and renders after every write and in `todo_get_task_list`.
- (slices) The sidebar, token-count, context-assembly, and preferences actors are trouper ServiceActors spawned at slice activation.
- (slices) The chat input box cannot be remotely locked or disabled.
- (todo) The todo auto-prune worker force-includes the most recent `todo_*` tool loop and excludes all older ones, so exactly one current task list stays in context; user pins and `x` toggles supersede it.
- (pickers) The task-list picker browses phases and tasks as a tree, hides postponed tasks, and Enter is a no-op.
- (skills) jinn ships a bundled `jinn-usage` agent skill whose body routes to per-topic reference files (keybindings, workflows, configuration) installed beside its SKILL.md.
- (skills) Bundled skill content is compile-time embedded, so installed skill docs match the running jinn binary; refreshing them requires `jinn install --force`.
- (slices) The sidebar's section focus is a dynamic scope per section (sidebar/<section>); FocusScope and the TUI Scope have no static sidebar variants.
- (slices) A sidebar section is derived from the dynamic scope id's name; ScopeStack is_sidebar and sidebar_section match scope ids with the sidebar slice prefix.
- (tools) interactive_term_send and interactive_term_kill carry the calling chat session id and act only on that session's own terminal; no model-facing terminal id argument exists.
- (tools) Terminal control (user takeover) is tracked per chat session; a takeover or handback in one session never affects another session's in-flight interactive_term calls.
- (tools) Closing a session kills its live interactive_term terminal; the coordinator subscribes to SessionClosed.
- (tools) The interactive_term tool guidance warns models not to append shell redirections, pipes, or grep (the tool returns the rendered screen, so piped output is silently lost) and advertises the no-argument interactive_term_send call as an anytime screen snapshot; the usage footer on every result repeats both.
- (tools) The jinn-tools slice owns the tool orchestrator, the built-in and todo tools, the task subagent machinery, and the tool protocol contracts in jinn-tools-msg; tool nouns (ToolDefinition/ToolCall/ToolResult) live in jinn-core-types.
- (slices) Kernel feature extraction follows the absorb model: each slice family absorbs its feat/ modules, leaving jinn-domain as shared multi-slice vocabulary.
- (slices) The turn-dispatch slice is a crate owning the queue ServiceActor and the enqueue dispatch path; its wire contracts live in jinn-turn-dispatch-msg.
- (session) Forking a session persists the source session before forking, so the fork always reflects the source's current history and includes the entry it was forked from.
- (input) In the rename popup, ctrl+c clears the buffer and closes the popup when the buffer is already empty; escape always closes.
- (session) Pinning or unpinning a chat entry marks the session interacted, so the pin change persists even on a session that was never sent to.
- (preferences) The preferences and app-state actors handle UpdatePreferences and UpdateAppState through their trouper .handles declarations.
- (plugins) Existing `[plugin.*]` tables in a user's jinn.toml persist as unknown keys through config saves and are never read.
- (boot) The startup tail — the GetEnvironmentConfig ask and the EnvironmentLoaded publish — runs in composition after AllActorsSpawned, not in the slice.
- (session) Live session state is owned by the `jinn-session-state` crate, which preserves the authoritative atomic session aggregate and runtime turn state.
- (session) Durable session persistence uses a complete `SessionSnapshot` containing session metadata, history, task state, and token accounting.
- (session) The session turn reducer is owned by `jinn-session-turn` and coordinates history, phase, streaming, tools, retries, and persistence.
- (session) SQLite session persistence commits metadata, history, attachments, and token-ledger changes in one transaction.
