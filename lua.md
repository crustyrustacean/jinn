# Lua Workflow System — State, Gaps & Roadmap

## Current State

### Architecture

The Lua workflow system sits alongside the existing node-based workflow engine (`jinn-workflow`). It has two layers:

**`jinn-lua-workflow` crate** — VM infrastructure:

- `spawn_one_shot` — runs a Lua script on a `spawn_blocking` thread with a `LocalSet`. Loads the script, calls `run(ctx)`, returns the result as `serde_json::Value`.
- `CtxConfig` — builder declaring which capabilities the script gets: `llm`, `push_user`, `push_system`, `turn_off`, `gather`. Carries `session_id`, `workflow_id`, and optional `system_prompt`.
- `HostRequest` protocol — channel-based request/response between VM and domain layer. Variants: `Llm`, `PushUser`, `PushSystem`, `TurnOff`, `Shutdown`. Each carries a `oneshot::Sender`.
- `LuaRegistry` — maps plugin names to `VmHandle` structs (channel sender + join handle). Fully `Send + Sync`.

**`jinn-domain` integration layer:**

- `luaworkflow/host_handler.rs` — `LuaHostHandler` processes `HostRequest` variants against the domain layer. `PushUser`/`PushSystem` write entries to session history via `State`. `TurnOff` disables the attached workflow. `Llm` calls `DomainNodeContext::send_llm_request_cloned`. Runs as an async loop draining the channel until `Shutdown`.
- `workflow/workflow_controller_actor.rs` — `WorkflowControllerActor` bridges the actor bus to Lua execution. Subscribes to `SessionPhaseChanged`, `AttachWorkflow`, `DetachWorkflow`, `ToggleWorkflow`, `TriggerWorkflow`, `FireBeforeTurn`. Dispatches `WorkflowConfig::Judge` to `spawn_lua_workflow`, which reads the script from disk, constructs a `CtxConfig`, calls `spawn_one_shot`, and runs the handler loop alongside the VM task.
- `workflow/attached_workflow.rs` — `WorkflowConfig::Judge` has a `script: String` field (defaults to `"judge_fail"`) that selects which Lua plugin to run.

### Communication model

```
Lua VM (spawn_blocking thread)
  │
  │ kanal::Sender<HostRequest>  (sync, unbounded)
  ▼
Host Handler (tokio task)
  │
  ├── PushUser/PushSystem → State.write() → session.push_entry()
  ├── TurnOff → State.write() → set enabled=false, state=Completed
  ├── Llm → DomainNodeContext::send_llm_request_cloned (async)
  │         → clones session → enqueues message → awaits oneshot
  │         → WorkflowActor resolves oneshot on SessionPhaseChanged(Idle)
  └── Shutdown → break loop
```

Each capability call is synchronous from Lua's perspective (async functions yield via `LocalSet` until the response arrives). The host handler processes requests in a loop with 10ms polling intervals.

### Capabilities available to scripts

| Capability | Lua API | Domain effect |
|---|---|---|
| `llm` | `ctx.llm(prompt)` → string | Clones session, sends to LLM, awaits response |
| `push_user` | `ctx.push_user(text)` | Writes `ChatEntry::user` to session history |
| `push_system` | `ctx.push_system(text)` | Writes `ChatEntry::system` to session history |
| `turn_off` | `ctx.turn_off()` | Disables the attached workflow |
| `gather` | `ctx.gather({fn, fn, ...})` | Runs functions concurrently, returns results table |

### Plugin loading

Scripts live on disk as `plugins/<name>/init.lua`. Two search paths, user takes priority:

```
~/.config/jinn/plugins/<name>/init.lua     (user plugins)
/usr/share/jinn/plugins/<name>/init.lua    (system plugins)
```

Scripts are read from disk at invocation time. No hot-reload mechanism needed — edits take effect on the next trigger.

### Existing scripts

```
res/plugins/
  judge_fail/
    init.lua    — ctx.push_user("judgement failed, try again")
  judge_pass/
    init.lua    — ctx.push_system("judgement passed"); ctx.turn_off()
```

### Test coverage

- **6 unit tests** on `LuaHostHandler` (push_user, push_system, turn_off, error cases, run loop)
- **3 integration tests** on `WorkflowControllerActor`:
  - `judge_fail_pushes_user_entry_to_session` — direct `spawn_lua_workflow` call
  - `judge_pass_pushes_system_entry_and_disables_workflow` — direct call with different script
  - `turn_end_trigger_fires_judge_fail_lua_workflow` — full event path via `handle_session_phase_changed`

### What works end-to-end

- Lua script loads and executes
- `ctx.push_user()` / `ctx.push_system()` write entries to session history
- `ctx.turn_off()` disables the attached workflow
- `ctx.llm()` wired through `DomainNodeContext` to cloned-session LLM calls
- Script resolution: user plugins dir → system plugins dir fallback
- `WorkflowConfig::Judge` parameterizes which script to run
- TurnEnd trigger fires Lua workflows when session goes Idle
- Attached workflows appear in the session sidebar as child entries

---

## Gaps

### 1. Controller not spawned at startup

`WorkflowControllerActor` is fully implemented and tested but never registered in the actor host's startup sequence. Nothing runs without this.

### 2. No plugin discovery

The controller reads scripts from disk on demand (when a workflow fires) but has no startup-time scan of available plugins. The picker cannot populate Lua entries because nothing knows which scripts exist.

### 3. No `WorkflowConfig::Lua` variant

Only `WorkflowConfig::Judge` routes to `spawn_lua_workflow`. Attaching an arbitrary Lua script requires pretending it's a Judge. A generic `Lua` variant would let any plugin be attached without semantic mismatch.

### 4. No picker integration

The workflow picker (`WorkflowActor::handle_load_workflow_picker_entries`) only shows entries from the node-based `WorkflowRegistry`. It doesn't know about Lua plugins. The user has no way to discover or attach a Lua workflow from the UI.

### 5. Silent script errors

When a Lua script fails (syntax error, runtime error, missing file), the error is logged via `tracing::error!` but never surfaced to the user. The attached workflow state stays `Running` indefinitely.

### 6. Vague sidebar labels

Attached Lua workflows show as "Judge" in the sidebar — no indication of which script is running. Multiple attached scripts are indistinguishable.

### 7. No cancellation on detach/disable

Disabling or detaching a workflow leaves the VM task running to completion. The `Shutdown` host request exists but is never sent.

### 8. No concurrent execution guard

If a `TurnEnd` trigger fires rapidly, multiple VMs spawn concurrently, all writing to the same session.

### 9. No script context for reading session data

Scripts can push entries into the session but cannot read from it. No way to access the last assistant message, conversation history, or config parameters.

---

## Roadmap

Ordered by dependency. Items 1–5 are the minimum for a user to pick, attach, and run a Lua workflow from the UI.

### 1. Spawn controller at startup

**Unblocks:** Everything.

**What:** Find the startup code that spawns `WorkflowActor` and add `WorkflowControllerActor` alongside it. Wire its deps (`state`, `services`).

**Effort:** Small.

### 2. Plugin discovery

**Unblocks:** Picker integration.

**What:** Add a `discover_plugins(paths: &AppPaths) -> Vec<PluginMeta>` function that scans both `plugins_dir()` and `system_plugins_dir()` for directories containing `init.lua`. Deduplicate (user overrides system). Store results in a `LuaPluginRegistry` accessible to the controller and picker. Derive name from directory name, description from a first-line comment convention or `description.txt` if present.

**Effort:** Medium.

### 3. `WorkflowConfig::Lua` variant

**Unblocks:** Picker integration, init path.

**What:** Add `Lua { script: String }` to `WorkflowConfig`. Update `spawn_lua_workflow` to route both `Judge { .. }` and `Lua { .. }` to the Lua execution path. Update all match guards and construction sites.

**Effort:** Medium.

### 4. Picker integration

**Unblocks:** User-facing usability.

**What:** Extend the picker entry loader to also iterate discovered Lua plugins. When the user selects a Lua entry, emit `AttachWorkflow` with `WorkflowConfig::Lua { script }` and the chosen trigger. The trigger selection can default to `TurnEnd` for now.

**Effort:** Medium.

### 5. Error reporting

**Unblocks:** Trust in the system.

**What:** In `spawn_lua_workflow`, when the VM errors or the script is not found, push a `ChatEntry::system("⚠ script error: ...")` into the session and set the attached workflow state to `Failed { message }`.

**Effort:** Small.

### 6. Sidebar labels

**What:** Extend `label_or_default()` to include the script name — e.g., `"Judge (judge_fail)"` or just `"judge_fail"`. Makes multiple attached scripts distinguishable.

**Effort:** Small.

### 7. Cancellation

**What:** Track the `host_tx` channel sender per running workflow in the controller. When `DetachWorkflow` or `ToggleWorkflow(disable)` arrives, send `HostRequest::Shutdown`. The handler loop already breaks on `Shutdown`.

**Effort:** Medium.

### 8. Concurrency guard

**What:** Track `running_scripts: HashSet<WorkflowId>` in the controller. Before spawning, skip if already running. Clear when the join handle resolves.

**Effort:** Small.

### 9. Script context for reading session data

**What:** Add capabilities: `ctx.last_assistant_message()`, `ctx.history(count)`, `ctx.get_config(key)`. Each maps to a new `HostRequest` variant that reads from `State`.

**Effort:** Small per capability.
