# Phase 1: Foundation

## Problem

We need to introduce kameo and kameo_actors into the project, create the BusMessage marker trait, build the kanal closure bridge for sync→async message delivery, update IntentResult, and wire everything into startup — all while coexisting with the existing actor system.

## What Moves / What Stays

- **Stays**: All existing actor infrastructure (Actor trait, InMemoryActorHost, Command/Event enums, ActorEnvelope, etc.)
- **Stays**: All existing actors, handlers, tests — completely untouched
- **Moves**: IntentResult gains a new closure-based API but keeps the old fields temporarily
- **New**: BusMessage trait, bridge module, kameo deps, MessageBus spawn in startup

## File Changes

### 1. Modified: `Cargo.toml` (workspace root)
Add kameo and kameo_actors to workspace dependencies.

### 2. Modified: `crates/jinn-domain/Cargo.toml`
Add kameo and kameo_actors as dependencies.

### 3. Created: `crates/jinn-domain/src/common/bus.rs`
BusMessage marker trait:
```rust
pub trait BusMessage: Clone + Send + 'static {}
```

### 4. Created: `crates/jinn-domain/src/common/bridge.rs`
Kanal closure bridge connecting sync TUI to async kameo bus.

### 5. Modified: `crates/jinn-domain/src/common/mod.rs`
Export new `bus` and `bridge` modules.

### 6. Modified: `crates/jinn-domain/src/common/services.rs`
Add `bus` field to `Services` struct (wrapped in Option initially for coexistence).

### 7. Modified: `crates/jinn-domain/src/protocol/intent.rs`
Add `with_message` helper to IntentResult alongside existing fields.

### 8. Modified: `src/actor_wiring.rs`
Spawn MessageBus, create Bridge, inject into Services and AppCore.

### 9. Modified: Message struct files for spike actors
Add `impl BusMessage` to each message struct used by EchoActor, DiscoveryNotifier, QueueActor.

## Implementation Order

1. Add dependencies to Cargo.toml files
2. Create bus.rs (BusMessage trait)
3. Create bridge.rs (kanal closure bridge)
4. Export new modules in common/mod.rs
5. Add bus field to Services
6. Update IntentResult with closure helpers
7. Wire startup (spawn bus, create bridge, inject)
8. Add impl BusMessage to spike actor message types
9. Write tests

## Acceptance Criteria

- [x] `cargo check --workspace` passes with kameo deps added
- [x] BusMessage trait exists and is exported from common
- [x] Bridge module exists, compiles, provides sync sender + async drain
- [x] Services struct has bus and bridge fields
- [x] IntentResult has with_message helper
- [x] MessageBus spawns during startup (with handle.enter() guard)
- [x] Phase 1 tests pass (bus routing, end-to-end bridge)
- [x] All existing tests still pass (old system untouched)
- [x] All e2e scenarios pass (10 headless + 32 app + 2 bench)


## Changes

- Added `kameo` v0.20 and `kameo_actors` v0.4 as workspace dependencies
- Created `BusMessage` marker trait in `common/bus.rs` — any `Clone + Send + 'static` type can implement it
- Created `Bridge` struct in `common/bridge.rs` — wraps a kanal unbounded channel carrying `Box<dyn FnOnce(&ActorRef<MessageBus>) + Send>`. The sync `BridgeSender` is used by TUI; the async `BridgeReceiver` drains in a tokio task and calls each closure with the bus ref.
- Added `BusService` wrapper in `common/bus.rs` wrapping `ActorRef<MessageBus>`
- Added `bus: Option<BusService>` and `bridge: Option<BridgeSender>` fields to `Services`
- Added `IntentResult::with_message()` and `with_closure()` helpers
- Spawned `MessageBus` in `actor_wiring.rs::build()` inside `handle.enter()` guard
- Injected bridge and bus into `AppCore`
- TUI test world (`TuiWorld`) sets bus/bridge to `None` (not needed for TUI tests)
- App test world (`AppWorld`) gets bus via production `ActorSystemBuilder::build()`
- Added 2 tests in `bridge.rs`: bus routing and end-to-end bridge closure delivery

## Divergence

- The `IntentResult` migration (replacing `commands`/`events` fields) is deferred to phase 4 (big bang). The current `with_message` helper is additive.
- The `handle.enter()` guard was needed for kameo's `tokio::spawn` calls in `build()`. This is correct because `build()` runs on a thread that has the runtime but isn't entered.

## Verification

- `cargo check --workspace` passes
- `cargo test --workspace` — all 3700+ unit tests pass, 0 failures
- `just e2e` — all 44 scenarios pass (10 headless + 32 app + 2 bench)
- Phase 1 bridge tests pass in `jinn-domain`

## Risks

- The `MessageBus` from kameo_actors is relatively new. If it has performance issues under high throughput, we may need to benchmark it in later phases.
- The closure bridge adds one `tokio::spawn` per message. For high-throughput paths (streaming tokens), this could add overhead. We should monitor this during the spike (phase 2).
