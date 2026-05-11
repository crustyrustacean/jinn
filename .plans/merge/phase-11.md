# Phase 11: Refactor spawning

## Problem

`src/app.rs` manually creates channels, contexts, and activates actors for each slice. Each slice should export a `spawn()` function that encapsulates this boilerplate, simplifying `app.rs`.

## What Moves

Channel creation, context setup, data injection, and actor activation for each actor-bearing slice → `spawn()` function in each slice's `lib.rs`.

## What Stays

- `create_core_with_actor_host()` stays in `src/app.rs` but calls `spawn()` functions
- Actor lifecycle event emission stays in `app.rs` (it needs to know actor names)

## File Changes

### 1. MODIFY `crates/slices/nsslice-echo/src/lib.rs`
Add:
```rust
pub fn spawn(
    sink: Arc<dyn MessageSink>,
    handle: &tokio::runtime::Handle,
) -> (ActorRef<EchoDirectMsg>, ActorResult) {
    let (tx, rx) = kanal::unbounded::<ActorEnvelope<EchoDirectMsg>>();
    let actor_ref = ActorRef::new(tx);
    let mut ctx = ActorContext::new("echo", sink.clone());
    ctx.set_description("Echoes messages back");
    let actor = EchoActor::activate(&mut ctx);
    spawn_actor("echo", actor, &actor_ref, rx, ctx, handle)
}
```

### 2-7. Same pattern for: `nsslice-shutdown`, `nsslice-llm`, `nsslice-tools`, `nsslice-provider` (2 actors), `nsslice-session-management`, `nsslice-context` (2 actors)

### 8. MODIFY `src/app.rs`
Replace inline channel/context/activation code with calls to `spawn()`.

- [x] Each actor-bearing slice exports a `spawn()` function
- [x] `src/app.rs` calls `spawn()` functions instead of inline setup
- [x] `just check` passes
- [x] `just test` passes

---

## Review: Phase 11 — Refactor spawning

### Changes

- Added `spawn()` functions to all 7 actor-bearing slices: `nsslice-echo`, `nsslice-shutdown`, `nsslice-llm`, `nsslice-tools`, `nsslice-provider` (2 functions), `nsslice-session-management`, `nsslice-context` (2 functions)
- Refactored `src/app.rs::create_core_with_actor_host()` to call spawn functions instead of manually creating channels, contexts, and activating actors
- Removed unused imports from `app.rs` (Actor, individual actor types, ActorContext, ActorEnvelope, ActorRef, spawn_actor)

### Divergence Summary

- Function names differ slightly from plan: `spawn_provider_actor`/`spawn_discover_actor` instead of a single `spawn()`, `spawn_context_actor`/`spawn_prompt_scan_actor` for context, `spawn_session_actor` for session. This is because these slices have multiple actors.
- Return type is `ActorSpawnResult` not `ActorResult` as in the plan — the codebase uses `ActorSpawnResult`.

### Verification

- `just check` — zero errors, zero warnings
- `just test` — all pass

### Risks

- None.

### Next Steps

Phase 12 (dissolve actors/) is already complete. Proceed to Phase 13: Final cleanup.
