# Architecture Note: Plugin-dispatch actor must not await self-cyclic work

**Status:** Resolved (enrichment path — Phase 1; lifecycle path — Phase 3).
**Purpose:** Permanent record of the invariant and the bug it prevented, so the pattern
isn't reintroduced by a future change.

## The invariant

**Actor handlers must not await work whose completion depends on a message to the same
actor.** The `PluginDispatchActor` runs one message at a time (`actor.handle(envelope).await`
in `in_memory.rs`). Any handler that `.await`s a condition whose resolution routes back
through this actor's mailbox will self-deadlock until the 30-second `AsyncPluginHandle`
timeout fires, after which the stuck message finally processes and the result "leaks
through" ~5ms late.

## How it manifested

The enrichment plugin's `on_enrich` hook awaited `ctx.request("llm_oneshot")`, which awaits
the one-shot session reaching `Idle`. The `SessionPhaseChanged(Idle)` event is delivered to
`PluginDispatchActor` — but the actor was still blocked in `handle_fire_async_hook` awaiting
`on_enrich`. The event sat in the mailbox; the oneshot never resolved; 30s later the
`AsyncPluginHandle` timeout fired → `PluginFireError`; the actor freed; the queued event ran;
`resolve_completed` fired; the text landed ~5ms after the error. Symptom looked like a "slow
model" but the model had finished in 3-4s.

## The fix pattern

All fire methods on `PluginDispatchActor` route through `spawn_fire_for_session`, which
resolves the registry id synchronously and then `tokio::spawn`s the fire onto a background
task:

- `handle_fire_async_hook` (enrichment path)
- `fire_on_app_started`
- `fire_on_session_created`
- `fire_on_phase_changed` — note: the synchronous `resolve_completed` block runs **before**
  the spawn. That ordering is load-bearing (it's how plugin LLM one-shots resolve); only the
  trailing fire is spawned.

The plugin-async thread still serializes Lua execution internally (its own single-threaded
loop + channel), so spawning at the actor level introduces no Lua concurrency hazard.

## What is NOT a deadlock (and why)

`handle_attach` / `handle_detach` / `handle_toggle` also `.await` calls to the plugin thread
(`create_session_registry` / `destroy_session_registry`), but those just allocate/drop a Lua
state — they don't run any user hook, so nothing can call `ctx.request` and await a
dispatch-routed event. They're a latency cost (the actor blocks ~50ms), not a correctness
bug. They were left as-is.

## The guard for future work

Treat any `async fn` on an actor whose body awaits a service that round-trips through the
same actor as a code-review red flag. The `send_llm_request_oneshot` →
`SessionPhaseChanged(Idle)` → `resolve_completed` cycle is the canonical example; any new
`ctx.request` name whose handler emits a command the dispatch actor subscribes to is another.

The 30-second timeout in `async_handle.rs` remains as a **safety net** for a genuinely stuck
plugin thread, but it must never be the thing masking a structural bug.
