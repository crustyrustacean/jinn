# Architecture

nullslop is a TUI chat application built on an intent-driven architecture with an actor bridge. User input flows through a single `IntentHandler` that validates, mutates shared state, and emits domain commands. Domain-specific actors run asynchronously, communicate through the actor host's pub/sub routing, and write their state fields directly into shared `AppState`. There is no bus.

The application supports two execution modes — an interactive TUI and a headless mode. Headless mode has two paths: a script runner that parses keystroke sequences through the same which-key and IntentHandler pipeline as the TUI, and a send-chat shortcut that bypasses the IntentHandler entirely and sends commands directly to the actor bus. Both modes share the same actor wiring and coordinated shutdown.

## Data Flow

```
  Keyboard / Mouse / Script
         │
         ▼
  Keymap (produces Intent)
         │
         ▼
  IntentHandler (sync, single match block)
    ├── validator  → passes or rejects
    ├── mutate AppState directly (scroll, cursor, mode, etc.)
    └── return IntentResult { commands }
         │
         ▼
  AppCore.sender  →  async forwarding task  →  ActorHost
                                                    │
                                          ┌─────────┴─────────┐
                                          │                   │
                                   Domain actors          other actors
                                          │                   │
                                          ▼                   ▼
                                    write AppState        Commands/Events
                                    (shared RwLock)             │
                                          │                    │
                                          └─────────┬──────────┘
                                                    ▼
                                            TUI renderer reads AppState
```

### Message flow

```
Frontend ──Command──▶ Actor Host ──▶ Subscribed Actors
                                        │
                                        ▼  (actor-to-actor via same pub/sub)
                                   Domain actors write AppState (RwLock)
                                        │
                                        ▼
                                   Renderer reads AppState (next tick)

Unidirectional: frontend → actor system. No feedback loop.
The shared AppState is the feedback — domain actors write their fields,
the renderer reads it on the next tick.
```

## Intent Handler

The `IntentHandler` is the single decision point for all user input. Its `handle` method takes an `Intent`, validates it, mutates `AppState` directly, and returns an `IntentResult` carrying zero or more domain `Command`s:

```rust
pub fn handle(intent: &Intent, state: &mut AppState) -> IntentResult
```

An intent does exactly one of three things:

1. **Fail validation** → no state change, no commands
2. **Mutate `AppState` directly** (pure UI: scroll, cursor, mode switch) → no commands
3. **Mutate `AppState` + emit `Command`s** (domain: submit message, switch provider) → commands forwarded to the actor system

The `IntentHandler` never accesses external services. It never emits events. It only reads/writes `AppState` and returns commands for the caller to send.

## Validators

Each intent has a dedicated validator function. Validators are plain functions — no registries or trait objects:

- **Infallible intents** — always succeed, no validation logic needed. These don't need a validator.
- **Fallible intents** — return `Result<(), SpecificError>` with a custom error enum per intent.

All validators take `&AppState` as input. On validation failure, the `IntentHandler` match arm does nothing (no-op). Example:

```rust
pub fn validate_submit_message(state: &AppState) -> Result<(), SubmitMessageError> {
    if state.active_chat_input().is_empty() {
        return Err(SubmitMessageError::EmptyBuffer);
    }
    Ok(())
}
```

## Shared State

`AppState` is the single source of truth — one struct containing all UI and domain state. `State` wraps it in an `Arc<RwLock<AppState>>` for cross-thread access.

`AppState` is divided into five domain-grouped sub-structs, each owned by a specific subsystem:

- `SessionState` — owned by the session-persistence actor
- `ContextAssemblyState` — owned by the context actor (prompt templates, skills, personas, tool definitions)
- `ProviderState` — owned by the provider actor
- `FrontendState` — owned by the intent handler (tabs, pickers, scope stack, theme, notifications)
- `PluginSlotRegistry` — owned by the plugin actor (status bar slots)

Cross-boundary writes are a code review red flag.

Both the synchronous intent handler (on the main thread) and the async actor system share the same `State`:

- **Intent handler** — acquires write lock, validates, mutates, returns commands, releases lock
- **Domain actors** — acquire write lock per handler, mutate their owned fields, release lock, then emit
- **Renderer** — acquires read lock on each tick, draws to screen

`AppState` implements `Default` for easy test construction.

## Actors

Actors run asynchronously as tokio tasks. They communicate through the actor host's pub/sub routing — no direct bus required.

### Actor trait

Every actor implements the `Actor` trait:

```rust
trait Actor {
    type Message: Send + 'static;

    fn activate(ctx: &mut ActorContext) -> Self;
    fn handle(&mut self, msg: ActorEnvelope<Self::Message>, ctx: &ActorContext)
        -> impl Future<Output = ()> + Send;
    fn on_shutdown(&mut self, _ctx: &ActorContext) -> impl Future<Output = ()> + Send { ... }
    fn shutdown(self) -> impl Future<Output = ()> + Send where Self: Sized { ... }
}
```

- `activate` — constructor. Subscribes to events/commands, extracts peer refs, receives injected services via `ctx.take_data::<T>()`.
- `handle` — unified message handler for all incoming messages.
- `on_shutdown` — graceful cleanup inside the run loop when `ApplicationShuttingDown` is received. Runs before the loop exits. Default is a no-op.
- `shutdown` — final cleanup after the run loop exits. Takes ownership (`Self: Sized`).

The shutdown lifecycle has three phases: `on_shutdown()` is called when the shutdown system message arrives, the run loop auto-emits `ActorShutdownCompleted`, then `shutdown(self)` is called after the loop exits.

### ActorEnvelope

Every message an actor receives is wrapped in an `ActorEnvelope`:

- `Event(Event)` — an event the actor subscribed to
- `Command(Command)` — a command the actor registered for
- `Direct(M)` — typed direct message from another actor (the actor's own message type)
- `System(SystemMessage)` — lifecycle broadcast. Currently only `ApplicationShuttingDown`.

### Subscriptions

During `activate`, an actor declares what it cares about:

```rust
ctx.subscribe_event::<SomeEvent>();
ctx.subscribe_command::<SomeCommand>();
```

The actor host builds pre-computed routing tables from these declarations. Events are broadcast to all subscribers. Commands are routed to exactly one subscriber — subscribing multiple actors to the same command panics at startup.

### Source filtering

Actors never receive their own emissions. The actor host passes the originating actor's name when routing, and skips the source. This prevents echo loops without requiring actors to filter their own messages.

### ActorRef

Each actor gets a typed, cloneable `ActorRef<M>` handle wrapping a channel sender. `ActorRef` is used for:

- Direct actor-to-actor messaging (via `send(M)`)
- Captured in routing table closures for pub/sub delivery
- Swappable sender for restart scenarios

### ActorContext

Provided to all actor methods. Holds:

- Actor name and description
- Subscription accumulation (`subscribe_event::<T>()`, `subscribe_command::<T>()`, `subscribe_all_events()`)
- Type-keyed peer `ActorRef` storage (for direct messaging between actors)
- Type-keyed data injection (`set_data::<T>()` / `take_data::<T>()`) — used to inject services during activation
- `MessageSink` access (for emitting commands/events back onto the bus)
- Lifecycle announcements (`announce_started`, `announce_shutdown_completed`)

### MessageSink

How actors emit back to the bus:

```rust
trait MessageSink {
    fn send_command(&self, command: Command) -> SendResult;
    fn send_event(&self, event: Event) -> SendResult;
}
```

A `RecordingSink` implementation records all messages in memory for tests.

### ActorHost

The host manages actor lifecycle and message routing:

- `send_event` — routes to all subscribed actors, skipping the source
- `send_command` — routes to the single registered actor, skipping the source
- `send_system` — broadcasts to all actors regardless of subscriptions
- `begin_shutdown` — initiates coordinated shutdown. Takes a oneshot sender that fires when all actors have completed shutdown. A `ShutdownTracker` counts active actors and signals completion when the count reaches zero.
- `shutdown` — awaits the coordinated shutdown completion

Two implementations exist: `InMemoryActorHost` (production, tokio tasks with pre-computed routing tables) and `FakeActorHost` (tests, records messages without spawning actors). Routing tables are built once at startup from actor subscriptions — the hot path is lock-free.

### Spawning

Actors are spawned centrally from a wiring module using two functions:

- `spawn` — creates the `ActorRef`, runs `activate` (which registers subscriptions), spawns the tokio task running the receive loop, auto-emits `ActorStarting`/`ActorStarted` lifecycle events, and returns an `ActorSpawnResult`.
- `system_spawn` — same as `spawn` but skips auto-lifecycle events. Used for infrastructure actors (e.g., the system-ready coordinator).

The host consumes all spawn results to build its routing tables once at startup.

## Services Container

`Services` is the DI container for the actor system — a struct of service wrappers (each wrapping `Arc<dyn Trait>`). Created once at startup and injected into actors via `ActorContext::set_data`/`take_data` during activation. Actors store it as a field for their lifetime.

The container holds the services actors need to do their work: filesystem paths, the tokio runtime handle, the actor channel sender, the LLM service factory, the provider registry, resolved API keys, config storage, the session store, the strategy registry, and user preferences storage.

The frontend and intent handler do NOT receive `Services` — they work with `AppState` directly.

For tests, a builder pattern creates `Services` with all-fake implementations, allowing selective overrides of specific services.

## Initialization Sequence

Startup is a three-phase event chain:

1. **Infrastructure** — A system-ready actor is spawned (via `system_spawn`, no auto-lifecycle). An `AllActorsSpawned` event signals it to begin checking readiness.

2. **Init chain** — Two init actors drive the bootstrap:
   - `env-init` self-schedules initialization, loads config and API keys from environment, then emits `EnvironmentLoaded`.
   - `provider-init` subscribes to `EnvironmentLoaded`, builds the provider registry from config, resolves the last-used model, and sends `ProviderSwitch` if applicable.

3. **Readiness gate** — After all actors have reported `ActorStarted`, the system-ready actor fires a oneshot signal. The main thread blocks (with a timeout) until this signal arrives, then dispatches initial scan commands (skills, personas, prompt templates).

The key insight: startup is event-driven. Init actors trigger each other through the actor host's pub/sub — there's no sequential init script.

## Domain Subsystems

### Session Persistence

The session-persistence actor is the central orchestrator of chat state — it has the widest subscription surface of any actor. It manages:

- **Session lifecycle** — loading sessions from SQLite, saving on changes, creating new sessions
- **Message flow** — receives `EnqueueUserMessage`, manages the assembly → LLM → stream → tool execution chain
- **Streaming** — processes `StreamToken` and `StreamCompleted` events, writing entries into the active session
- **Forking** — `SessionForkRequested` creates a branch from a specific point in chat history

Sessions persist across application restarts in a SQLite database with schema migrations.

### Context Strategy System

Prompt assembly uses a pluggable strategy pattern. Available strategies (e.g., passthrough, sliding-window, compaction) determine how chat history is trimmed and formatted before sending to the LLM.

A `StrategyRegistry` service provides strategy discovery, and a `StrategyFactory` creates instances from configuration. Strategies can be switched at runtime via `SwitchPromptStrategy`.

### Tools Pipeline

A tool-orchestrator actor manages the tool execution lifecycle:

- Tools are registered dynamically (not hardcoded) via `RegisterTools`
- The LLM actor sends `ExecuteToolBatch` when the model requests tool use
- Built-in tools include bash, read, write, edit, and others
- Tool results flow back as `ToolExecutionCompleted` events, allowing the LLM actor to continue the conversation

The orchestrator manages batch execution and supports cancellation.

### Persona System

Personas are customizable system prompt profiles loaded from Markdown files with TOML frontmatter. Each persona defines the LLM's identity and behavioral guidelines.

- A scan actor discovers personas from disk on startup and on demand
- The active persona's body text becomes the system prompt in prompt assembly
- Users switch personas through the sidebar and picker UI
- A seed persona is bundled into the binary and written on first run

### Skills & Prompt Templates

Two scan actors provide disk-based discovery:

- `SkillsScanActor` discovers agent skill definitions and emits `SkillsLoaded`
- `PromptScanActor` discovers prompt templates from the user's config directory and emits `PromptTemplatesLoaded`

Both follow the same pattern: subscribe to a rescan command, scan a directory in a spawned task, emit a loaded event. Templates support variable expansion.

### Preferences

User preferences are persisted to a `nullslop.toml` file. Two actors handle this:

- `preferences` — handles `UpdatePreferences` commands, writes to disk
- `preferences-sync` — subscribes to `PreferencesUpdated` events, syncs values into `AppState.frontend.preferences`

This two-actor pattern separates persistence from state synchronization.

## Rendering

UI elements implement a `UiElement<S>` trait. They read state and draw to a ratatui `Frame`. Elements have no knowledge of intents, commands, or events — they just read state and draw.

```rust
trait UiElement<S> {
    fn name(&self) -> String;
    fn render(&mut self, frame: &mut Frame, area: Rect, state: &S);
    fn is_selectable(&self) -> bool;
}
```

Elements are registered into a `UiRegistry` at startup. The renderer iterates registered elements, allocates screen areas, and calls `render` with the current `AppState`.

## Keymap & Scopes

The keymap binds physical keys to `Intent` variants. Scope determines which bindings are active — it's derived from a LIFO `ScopeStack` of focus states (normal, input, sidebar, picker). The top of the stack determines the active keymap scope, which is cross-producted with the active tab (chat, dashboard) for the final binding set.

Three related but distinct concepts:

- **FocusScope** (domain layer) — the actual focus state: Normal, Input, Sidebar, Picker. Stacked with push/pop semantics.
- **Scope** (keymap layer) — derived from FocusScope + active tab. Determines which keybindings are active.
- **Mode** (display layer) — simplified view (Normal, Input, Picker) for UI rendering.

Multi-key sequences (e.g., `gg`, `gmr`) are supported with intermediate which-key prompts. Bindings are grouped by category in the which-key popup. Input and Picker scopes use catch-all handlers so unmapped character keys pass through as text input.

When a key matches, the which-key system produces an `Intent` and feeds it into the `IntentHandler`.

## Testing Strategy

- **Intent handler** — call `IntentHandler::handle(intent, &mut state)` directly, assert state changes and returned commands
- **Validators** — call in isolation with known state, assert `Ok`/`Err`
- **UI elements** — render with ratatui `TestBackend`, assert buffer contents
- **State types** — unit tests for behavior (grapheme handling, cursor bounds, etc.)
- **Actors** — send command/event, use `RecordingSink` to assert emitted messages

Tests follow Given/When/Then structure. See `AGENTS.md` for detailed testing patterns.
