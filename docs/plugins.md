# Plugin System Reference (Agent)

This document is the **authoritative reference for AI agents** touching the jinn
plugin system. It maps Rust touchpoints to Lua-side propagation requirements so
agents can keep both sides in sync.

Human-facing Lua authoring guide is in `res/plugins/meta/plugin_ctx.lua`
(LuaCATS annotations) and `res/plugins/meta/plugin_template.lua` (scaffold).

---

## 1. Overview

Plugins are Lua scripts that hook into app lifecycle events. Two scopes:

- **Global plugins** (`res/plugins/global/<name>/init.lua`) — loaded once at
  startup, fire for every session.
- **Attachable plugins** (`res/plugins/attachable/<name>/init.lua`) — loaded
  into a per-session Lua state only when attached; fire only for that session.

A plugin's `init.lua` returns a module table containing zero or more hook
functions. The plugin subscribes to events _by defining the matching hook
function_. There is no trigger enum, no manifest beyond a `-- description:`
header.

---

## 2. Directory Layout

```
res/plugins/
├── global/         # Always loaded at startup, fire for every session
│   └── welcome/
│       └── init.lua
├── attachable/     # Loaded on attach per session
│   ├── judge_fail/
│   │   └── init.lua
│   └── judge_pass/
│       └── init.lua
└── meta/           # LuaCATS type definitions (not loaded by runtime)
    ├── plugin_ctx.lua
    └── plugin_template.lua
```

User-installed plugins live in a parallel user dir; user overrides system
within kind. User dir defaults to `<config_dir>/plugins/`. See
`discover_plugins` in `crates/jinn-plugin/src/loader.rs`.

---

## 3. Hook Lifecycle

Hooks are fired by `PluginDispatchActor`
(`crates/jinn-domain/src/feat/plugin_dispatch/actor.rs`) in response to
lifecycle events. The actor is a thin event → hook dispatcher; it has no
trigger enum and no merge strategies. Plugins self-orchestrate via `ctx.emit`.

| Lifecycle event                  | Hook fired           | Scope                 | Ctx source                           |
| -------------------------------- | -------------------- | --------------------- | ------------------------------------ |
| `AllActorsSpawned`               | `on_app_started`     | Global only           | `fire_on_app_started` (line 330)     |
| `SessionCreated`                 | `on_session_created` | Global + that session | `fire_on_session_created` (line 344) |
| `SessionPhaseChanged(Sending)`   | `on_user_submit`     | Global + that session | `fire_on_phase_changed` (line 352)   |
| `SessionPhaseChanged(Idle)`      | `on_turn_end`        | Global + that session | `fire_on_phase_changed` (line 352)   |
| `SessionPhaseChanged(Streaming)` | (none)               | —                     | mid-turn; no hook fires              |

**Adding a new hook requires three updates**: the actor handler, the LuaCATS
meta file, and any existing Lua plugins that should opt in. See §10.

### Sync hooks (render thread)

Two sync hooks are fired directly from the render thread (not via the actor).
They use the `PluginSyncHooks` trait (`call_hooks` / `call_hooks_typed`) — see §5.
Their ctx is built by the call site (`IntentHandler::handle`, the chat-input
renderer) and round-trips through JSON into the sync Lua state.

| Fire site                                  | Hook fired                     | Scope    | Ctx source                          |
| ------------------------------------------ | ------------------------------ | -------- | ----------------------------------- |
| `IntentHandler::handle` (after a **submit-family** intent resolves) | `on_submit_intercept` | Global | `IntentHandler::handle` — fired only for `Intent::SubmitMessage`; other intents pass through unintercepted |
| Chat-input renderer (`chat_tab.rs`)        | `on_chat_input_badges_render`  | Global   | input-area render call site         |

`on_submit_intercept` lets a plugin block or replace a submit intent's resolved
commands. The wire shape is `{ action = "block" }` (drop the commands),
`{ action = "pass" }` (no-op), or `{ action = "replace", commands = {...} }`
(swap in new commands) — the same tags as the Rust `InterceptOutcome` enum.
It fires only for `Intent::SubmitMessage`; other intents (insert-char, quit,
scroll, …) are never intercepted. This is a deliberate scoping choice — the
original generic-over-all-intents design caused a keystroke flood once a toggle
was armed.
`on_chat_input_badges_render` lets a plugin return a **single** badge directive drawn
right-aligned on the input box's bottom border row (same row as the `[QUEUE]`/`[STEER]`
mode badge), or `nil` when there is nothing to draw. The directive shape is
`{ slot, segments = { { text, style? } } }`: an ordered list of styled runs rendered
left-to-right. `style` is a string from the badge style vocabulary — flat ratatui colors
(`"yellow"`, `"cyan"`, `"green"`, `"red"`, `"bold"`) or theme-derived colors
(`"accent_action"` for hotkeys, `"muted_text"`). The badge ctx also carries `mode`
(the current scope mode as a lowercase string); plugins branch on it to apply their own
styling (the host applies no mode-aware styling to plugin content).

### Plugin-defined async action hooks

A plugin may also define its own named async hooks fired on demand via the
`fire_async_hook` verb (see §7) or the `Intent::TriggerPlugin` keybind path.
These are not lifecycle events; they are plugin-specific action handlers
invoked from the render thread and executed on the plugin-async thread.
Example: `prompt_enrichment` defines `on_enrich` (keybind action for `<M-e>`) which runs
an LLM one-shot rewrite of the current draft and writes it back via `set_chat_input`.
Its `on_chat_input_badges_render` hook returns a persistent `[Enrich]` hotkey legend
whose `E` uses `accent_action` only while in Input mode.

---

## 4. Per-Session Lua States

Each session with attached plugins owns its own async Lua state (a
`SessionRegistryId` in `AttachedPluginRegistry`). When `AttachPlugin` is
handled, the registry is destroyed and recreated with the updated plugin list.
When `DetachPlugin` empties a session's attachments, the registry is removed.

This is why attached plugins never fire for other sessions: each session's Lua
state literally doesn't contain them.

Global plugins live in one shared async Lua state, plus one shared sync Lua
state on the render thread.

---

## 5. Four Access Patterns

The host has four ways to call into plugins. The pattern is chosen by _who_
is calling and _whether they need results_.

| Who           | Return values? | Blocking?       | API                                                      |
| ------------- | -------------- | --------------- | -------------------------------------------------------- |
| Render thread | Yes            | No (direct Lua) | `app.plugins.sync_hooks("name")`                         |
| Actor         | No             | No (async)      | `services.plugins.fire_async_json("name", &ctx)`         |
| Actor         | Yes            | No (async)      | `services.plugins.fire_async_collect_json("name", &ctx)` |
| Actor         | Yes            | Yes (blocking)  | `services.plugin_sync.call_hooks_json("name", &ctx)`     |

All four produce the same per-hook Lua ctx. Sync calls run on the render
thread's Lua state. Async calls enqueue `PluginJob` onto a kanal channel
to the background `plugin-async` thread.

The session-scoped variants exist on `PluginFireService`:

- `fire_async_for_session_json(registry_id, hook, &ctx)` — global + that
  session's plugins, fire-and-forget.
- `fire_async_collect_for_session_json(registry_id, hook, &ctx)` — same
  scope, collects return values.

The render-thread sync path has no per-session variants. Sync plugins are
global only (the render thread doesn't swap Lua states per session).

### `PluginSyncHooks` — the domain-facing sync trait

The render-thread sync path is exposed to `jinn-domain` via the
`PluginSyncHooks` trait (`crates/jinn-domain/src/feat/plugin_dispatch/plugin_sync_hooks.rs`).
It is **not `Send`** (it fronts the `!Send` `SyncPlugins` Lua state), unlike the
actor-path traits (`PluginFire`, `PluginSyncCall`) which are `Send + Sync`.

```rust
pub trait PluginSyncHooks {
    fn call_hooks(&self, hook: &str, ctx: &serde_json::Value) -> Vec<serde_json::Value>;
}
pub fn call_hooks_typed<T: DeserializeOwned>(
    plugins: &dyn PluginSyncHooks, hook: &str, ctx: &serde_json::Value,
) -> Vec<T>;  // silent-drop malformed + warn!
```

The raw `call_hooks` returns `Vec<Value>` for object-safety (the trait is held
as `&dyn PluginSyncHooks`); the typed `call_hooks_typed::<T>` free function
deserializes each result and drops malformed ones with a `warn!` log (plugin
robustness). Call sites pick a hook + build ctx + loop the typed results.
Used by interception (`on_submit_intercept` → `InterceptOutcome`) and rendering
(`on_chat_input_badges_render` → `BadgeDirective`).

---

## 6. Ctx Fields

Built by `build_async_ctx` (`crates/jinn-plugin/src/async_thread.rs:393`) and
`build_sync_ctx` (`crates/jinn-plugin/src/sync_state.rs:185`). Every hook
receives the same shape.

| Field                    | Type             | Notes                                                                   |
| ------------------------ | ---------------- | ----------------------------------------------------------------------- |
| `session_id`             | string           | From the incoming ctx_json.                                             |
| `plugin_name`            | string           | Set by builder. Used by self-targeting emits like `disable_plugin`.     |
| `plugin_data`            | any (JSON value) | This plugin's entry from the shared `PluginData` store (a **snapshot** taken at hook entry); `null` if unset. For the live value after an `await`, async hooks use `get_plugin_data()`. |
| `emit(verb, data)`       | function         | Fire-and-forget domain command. Sync closure; pushes onto kanal.        |
| `request(name, data)`    | function         | Async coroutine; yields until the named handler responds.               |
| `set_plugin_data(value)` | function         | Replaces this plugin's entry in the shared `PluginData` store. Async-only.          |
| `merge_plugin_data(value)` | function       | Shallow-merges top-level keys into this plugin's `PluginData` entry (untouched keys preserved). Async-only. |
| `get_plugin_data()`      | function         | Returns this plugin's **current** entry from the live store (re-reads each call). Async-only; use after an `await` to see writes from other fires. |

Additional fields can be added to the ctx_json at the actor's fire site
(`fire_on_phase_changed` etc) or at the sync hook call site. Those flow into ctx
via the JSON-to-table conversion at the top of `build_async_ctx`. For example,
the sync `on_submit_intercept` ctx carries `input_text` (the current chat-input
draft), set by `IntentHandler::handle` before firing the hook.


**LuaCATS source of truth**: `res/plugins/meta/plugin_ctx.lua`. Per-hook
subclassing (e.g., `OnTurnEndCtx : PluginCtx`) lives there.

---

## 7. Verb Catalog

Verbs are translated by `translate_command` in `src/plugin_wiring.rs`. Each
verb has a `Lua*` payload struct (with `#[derive(Deserialize)]`) and a
`From<Lua*> for Command` impl. Adding a verb is documented in §10.

Currently wired verbs:

| Verb                   | Lua payload                                                           | Rust command                                                                     |
| ---------------------- | --------------------------------------------------------------------- | -------------------------------------------------------------------------------- |
| `push_chat_entry`      | `{ session_id, kind: { system = "..." } }` or `{ transient = "..." }` | `Command::PushChatEntry`                                                         |
| `enqueue_user_message` | `{ session_id, text }`                                                | `Command::EnqueueUserMessage` (actually dispatches through LLM pipeline)         |
| `disable_plugin`       | `{ session_id, plugin_name }`                                         | `Command::TogglePlugin` (idempotent toggle; plugin can disable itself or others) |
| `fire_async_hook`      | `{ hook, session_id, ... }`                                            | `Command::Dynamic { name: "plugin::fire_async", payload }` (generic async handoff; actor subscribes by name and calls `fire_async_for_session_json`) |
| `set_chat_input`       | `{ session_id, text }`                                                 | `Command::SetChatInputText` (replaces the chat input box text for the session)   |

Kinds accepted by `push_chat_entry` are limited to `system`, `transient`, and
`error` via `LuaChatEntryKind` in `plugin_wiring.rs`. `user`, `assistant`,
`tool`, and other variants are **not** Lua-pushable — they have to go through
the proper constructors (`enqueue_user_message` for user entries, etc.).

`ctx.request(name, data)` resolves to handlers in `handle_plugin_request`
(`src/plugin_wiring.rs`). Currently wired request names:

| Request name   | Payload                           | Returns         |
| -------------- | --------------------------------- | --------------- |
| `llm_oneshot`  | `{ session_id, system, prompt, persist, disable_tool_loop, timeout_ms }`  | `{ ok: true, value: { text } }` on success, `{ ok: false, error }` on failure |

`ctx.request` always returns a **result envelope**: `{ ok = true, value = <response> }`
on success, or `{ ok = false, error = "<message>" }` on any failure (LLM error,
malformed payload, unknown request name). Hooks must inspect `result.ok` before
reading `result.value`. A failed request does not raise a Lua error; the error is
carried in `result.error` for the hook to surface (e.g. push a transient chat entry).

`llm_oneshot` runs a history-less LLM call inheriting only the session's
provider+model (no chat history). Used by prompt enrichment.

`persist` (optional, default `false`) controls whether the one-shot session is
written to the SQLite store. Transient by default; set `true` for reviewable
runs (e.g. a judge/eval). Either way the one-shot never becomes the active chat
view and (when `persist=false`) leaves no trace in history.

`disable_tool_loop` (optional, default `false`) controls whether the one-shot can
run tool loops. When `true`, the one-shot session runs with no declared tools and
`tool_loop_disabled` set, so the model cannot enter a tool loop (use this for pure
text rewrites like enrichment). When `false` (default), the one-shot inherits the
full tool catalog a normal session would see.

`timeout_ms` (optional, default `30000`) is a hard upper bound on the await. On
expiry the one-shot session is hard-cancelled (the active stream is aborted; no
background token burn) and the request returns an error to the hook.
written to the SQLite store. Transient by default; set `true` for reviewable
runs (e.g. a judge/eval). Either way the one-shot never becomes the active chat
view and (when `persist=false`) leaves no trace in history.

---

## 8. Plugin Data (`ctx.plugin_data` / `ctx.get_plugin_data` / `ctx.set_plugin_data` / `ctx.merge_plugin_data`)

Cross-context, in-memory only. Backed by `PluginData(Arc<DashMap<String, Value>>)`
in `crates/jinn-plugin/src/plugin_data.rs`. Keyed by plugin name.

- Async hooks write via `ctx.set_plugin_data(value)` (full replace) or `ctx.merge_plugin_data(value)` (shallow top-level merge; use it to update one field without a read-modify-write round-trip).
- Async hooks read **current** state via `ctx.get_plugin_data()` (re-reads the shared store; needed after an `await` so a hook observes writes from other fires — the `ctx.plugin_data` field is frozen at hook entry).
- Sync hooks read from `ctx.plugin_data` (auto-injected by `build_sync_ctx`; already current at entry since sync hooks never `await`).
- **Not persisted to disk.** Restarting the app wipes it.
- Each plugin sees only its own entry.

If persistence is needed in the future, this is the type to extend.

---

## 9. Error Story

| Failure                                                            | What happens                                                                                         | Where it surfaces                                                                 |
| ------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------- |
| Lua hook raises an error                                           | Hook run returns `Err(Report<PluginError>)` with the mlua error attached                             | Logged at `warn!` in `PluginDispatchActor::fire_for_session`; no chat-side effect |
| Plugin emits malformed payload (missing field, wrong type)         | `serde_json::from_value` fails in `translate_command`                                                | Logged at `error!` with `CmdCtx { plugin_name, verb }` + serde error attached     |
| Plugin emits unknown verb                                          | `translate_command` returns `Err(Report<PluginWiringError>)` with `CmdCtx` + "unknown verb" attached | Logged at `error!`                                                                |
| `sink.send_command(domain_cmd)` fails (e.g., actor mailbox closed) | `SendError` returned from sink                                                                       | Logged at `error!` with plugin_name + verb                                        |
| Plugin panics during hook                                          | Background thread's `LocalSet` catches it; oneshot dropped                                           | Caller gets `Err(Report<PluginError>)` with "thread died" context                 |

**Key invariant**: no plugin failure can lock up the actor system. The
`PluginDispatchActor` always `tokio::spawn`s hook fires and never awaits
collect results in its own `handle` method.

---

## 10. Cross-Cutting Invariants (Touch X → Also Update Y)

This is the table that keeps `docs/plugins.md`, the Rust code, and the Lua
types in sync. **Agents touching any row's left column must also touch the
right columns.**

| When you change...                                                         | Also update...                                                                                                                | Why                                                                 |
| -------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------- |
| Match arm in `translate_command` (`src/plugin_wiring.rs`)                  | (1) `res/plugins/meta/plugin_ctx.lua` — add verb to `PluginVerb` alias + new payload class. (2) This table's §7 verb catalog. | Lua authors get editor autocomplete on verb names + payload fields. |
| `Lua*` struct fields in `plugin_wiring.rs`                                 | `res/plugins/meta/plugin_ctx.lua` matching payload class                                                                      | Editor squiggles when Lua payload doesn't match.                    |
| `LuaChatEntryKind` variants in `plugin_wiring.rs`                          | `res/plugins/meta/plugin_ctx.lua` `ChatEntryKind` class                                                                       | Lua authors see which kinds are pushable.                           |
| `build_async_ctx` field additions in `async_thread.rs`                     | (1) `res/plugins/meta/plugin_ctx.lua` base `PluginCtx` class. (2) §6 ctx fields table.                                        | Editor shows new field; doc stays accurate.                         |
| New hook name in `PluginDispatchActor::handle_event`                       | (1) `res/plugins/meta/plugin_ctx.lua` — new `OnXxxCtx` subclass + entry in template. (2) §3 hook lifecycle table.             | Plugins can opt in; editor types the new ctx.                       |
| Hook ctx_json fields added at a fire site (e.g., `fire_on_phase_changed`)  | `res/plugins/meta/plugin_ctx.lua` matching `OnXxxCtx` subclass                                                                | Per-hook fields reflected in editor.                                |
| `PluginMeta` fields in `loader.rs` (e.g., new kind, new metadata)          | (1) §1 overview + §2 directory layout. (2) `scan_kind_dir` callers.                                                           | Discovery contract visible.                                         |
| `PluginCommand` struct in `crates/jinn-plugin/src/lib.rs`                  | `handle_plugin_command` in `src/plugin_wiring.rs` + §7 verb catalog                                                           | Cmd struct is the wire format between Lua and Rust.                 |
| `PluginData` semantics in `crates/jinn-plugin/src/plugin_data.rs`          | §8 plugin data section                                                                                                        | Persistence story stays accurate.                                   |
| `PluginDispatchActor` event subscriptions                                  | §3 hook lifecycle table                                                                                                       | Doc reflects which events fire which hooks.                         |
| `AttachedPlugin` struct (`crates/jinn-domain/src/feat/attached_plugin.rs`) | §4 per-session Lua states                                                                                                     | Attachment model documented.                                        |
| `PluginSyncHooks` trait (non-`Send`, `crates/jinn-domain/.../plugin_sync_hooks.rs`) | (1) §5 access patterns (sync direct vs channeled). (2) Any new sync hook name → §3 + `plugin_ctx.lua` `OnXxxCtx` subclass. | Sync hooks (interception, badge render) stay documented and typed.   |
| Sync hook name fired from `IntentHandler::handle` or a renderer        | (1) `res/plugins/meta/plugin_ctx.lua` — new `OnXxxCtx` subclass. (2) §3 hook lifecycle table. (3) The matching call site's ctx_json shape. | Sync ctx typed in editor; doc reflects fire source.                |
| `Intent::TriggerPlugin` variant or `KeyCategory::Plugin` (`crates/jinn-domain`/`jinn-tui`) | (1) `res/plugins/meta/plugin_ctx.lua` — document the `keybinds` table contract (keys, action, description, scope). (2) §3 hook lifecycle (plugin-declared action hooks fired via `fire_async_hook`). | Plugin keybind declaration contract visible to authors.             |
| `fire_async_hook` / `set_chat_input` verbs (`src/plugin_wiring.rs`)       | (1) `res/plugins/meta/plugin_ctx.lua` — `PluginVerb` alias + payload classes. (2) §7 verb catalog.                           | New verbs typed + catalogued.                                       |
| `llm_oneshot` request name (`src/plugin_wiring.rs::handle_plugin_request`) | `res/plugins/meta/plugin_ctx.lua` — document the request name + its `{ session_id, system, prompt, persist, disable_tool_loop, timeout_ms }` → `{ text }` contract.  | One-shot LLM request shape discoverable.                            |

---

## 12. Edge Cases & Gotchas

- **`push_chat_entry_transient` is gone.** Transient is now a kind variant
  (`{ kind = { transient = "..." } }`). Plugins using the old verb silently
  fail.
- **`ctx.plugin_name` must be used for self-disable.** Hardcoding the plugin
  name in the Lua payload works but is fragile across renames. The field is
  always set by `build_async_ctx`/`build_sync_ctx`.
- **`enqueue_user_message` actually re-dispatches the LLM.** Don't use it for
  insert-only messages. Use `push_chat_entry` with `kind = { system = "..." }`
  for those.
- **`disable_plugin` is idempotent.** It uses `Command::TogglePlugin`, so
  emitting it on an already-disabled plugin re-enables it. If you want strict
  "set to disabled," don't toggle; we'd need a new verb.
- **Lua errors carry the hook name and plugin name as `.attach(...)` context.**
  Always grep the log for the plugin name first.
- **Plugin discovery has a back-compat fallback.** If `global/` and
  `attachable/` don't exist at a root, `discover_plugins` does a flat scan
  treating everything as Global. New layouts should use the new dirs.

---

## 13. Navigation Anchors

Primary entry points when modifying the plugin system:

- **Verb wiring** — `src/plugin_wiring.rs::translate_command`
- **Plugin discovery** — `crates/jinn-plugin/src/loader.rs::discover_plugins`
- **Async hook execution** — `crates/jinn-plugin/src/async_thread.rs::run_single_hook`
- **Sync hook execution** — `crates/jinn-plugin/src/sync_state.rs::PluginHooks::call`
- **Ctx construction** — `crates/jinn-plugin/src/async_thread.rs::build_async_ctx` / `sync_state.rs::build_sync_ctx`
- **Plugin thread entrypoint** — `crates/jinn-plugin/src/async_thread.rs::async_thread_loop`
- **Actor dispatch** — `crates/jinn-domain/src/feat/plugin_dispatch/actor.rs::PluginDispatchActor`
- **Attachment model** — `crates/jinn-domain/src/feat/attached_plugin.rs::AttachedPlugin`
- **LuaCATS types** — `res/plugins/meta/plugin_ctx.lua`

---

## 14. Dependency Map

External:

- `mlua` — Lua 5.4 bindings; async via coroutines.
- `kanal` — channel between host and plugin thread; supports mixed sync/async.
- `dashmap` — backing store for `PluginData`.
- `error_stack` + `wherror` — error reporting (typed `Report<PluginError>` /
  `Report<PluginWiringError>` / `Report<PluginSyncStateError>`).
- `serde_json` — wire format for ctx, payloads, plugin data.

Internal:

- `jinn-plugin` — runtime (loader, async thread, sync state).
- `jinn-domain` — `PluginFire`/`PluginSyncCall` traits, `PluginDispatchActor`,
  `AttachedPlugin`, `Services` container.
- `jinn-cli` (via `src/plugin_wiring.rs`) — translates `PluginCommand` into
  domain `Command`s.
