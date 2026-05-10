# Style Guide

This document defines the _coding conventions_ and _patterns_ for the `nullslop` codebase. Always load the [ARCHITECTURE.md](./ARCHITECTURE.md) document for more detailed information that will help guide change requests and code reviews.

IMPORTANT NOTES:

- All async tasks are handled by actors. See ARCHITECTURE.md for more details.
- The `IntentHandler` is the single decision point for all user input — it validates, mutates state, and returns commands.
- Domain logic is handled by _sending a COMMAND_ to the actor system. This allows both sync & async workflows to occur from any point in the application.

## 1. Overview

This style guide ensures consistent, maintainable Rust code across the codebase. It covers error handling, trait-based design, testing patterns, documentation standards, and module organization. Following these patterns enables dependency injection for testability and clear separation of concerns.

## 2. Core Patterns

### Error Handling

Use `wherror::Error` with `error_stack::Report` for all fallible operations.

**Colocate errors with their related types.** Never create standalone `error.rs` or `errors.rs` files. Error types belong in the same module as the trait, struct, or function that produces them. For example, `ActorHostError` lives in `actor_host.rs` alongside the `ActorHost` trait, not in a separate `error.rs`.

**Error type:**

```rust
use wherror::Error;

#[derive(Debug, Error)]
#[error(debug)]
pub struct ExternalEditorError;
```

**Result with error context:**

```rust
use error_stack::{Report, ResultExt};

pub fn load() -> Result<Config, Report<ConfigError>> {
    let content = std::fs::read_to_string(&path)
        .change_context(ConfigError)
        .attach("failed to read config file")?;
    Ok(config)
}
```

**Document errors in functions:**

```rust
/// # Errors
///
/// Returns an error if the terminal setup fails.
pub fn run(tick_rate: Duration) -> Result<(), Report<TuiRunError>>
```

### Validator Pattern

Each `Intent` variant has a dedicated validator function. Validators are plain functions — no registries or trait objects. Fallible validators return `Result<(), SpecificError>` with a custom error enum per intent.

```rust
// Validator pattern — nullslop-intent/src/validators/
pub fn validate_submit_message(state: &AppState) -> Result<(), SubmitMessageError> {
    if state.active_chat_input().is_empty() {
        return Err(SubmitMessageError::EmptyBuffer);
    }
    Ok(())
}
```

**Validator rules:**

- All validators take `&AppState` as input
- Each fallible intent has a custom error enum describing why it cannot proceed
- On validation failure, the `IntentHandler` match arm does nothing (no-op)

### Trait Usage

Every external dependency or service must have a trait abstraction.

**Colocate traits with their related types.** Never create standalone `traits.rs` files. Traits belong in the same module as the types that implement them or the domain they define. For example, `MessageSink` lives in `message_sink.rs`, not in a separate `traits.rs`.

**Service trait pattern:**

```rust
use wherror::Error;

#[derive(Debug, Error)]
#[error(debug)]
pub struct FooBackendError;

pub trait FooBackend {
    fn fetch_all(&self) -> Result<Vec<Foo>, Report<FooBackendError>>;
}
```

**Service wrapper pattern:**

```rust
use std::sync::Arc;
use derive_more::Debug;

#[derive(Debug, Clone)]
pub struct ActorHostService {
    #[debug("ActorHost<{}>", self.backend.name())]
    host: Arc<dyn ActorHost>,
}

impl ActorHostService {
    pub fn new(host: Arc<dyn ActorHost>) -> Self {
        Self { host }
    }
}
```

**Key trait design rules:**

- Use `#[async_trait]` for async methods
- Include a `name(&self) -> &'static str` method for debugging on service traits
- Service structs wrap `Arc<dyn Trait>` for shared ownership

### Module Structure

**Workspace organization:**

```
Cargo.toml          # Workspace with members = ["crates/*", "actors/*"]
crates/
  nullslop/            # Main binary crate
    src/
      lib.rs
      main.rs
      app.rs
  nullslop-protocol/   # Command, Event, Intent, Mode, Key, AppMsg, CoreNotification
  nullslop-intent/     # IntentHandler, validators, validator errors
  nullslop-component-ui/    # UiElement trait, UiRegistry
  nullslop-component/       # State structs, UI elements, AppState, State
  nullslop-core/       # AppCore (state + sender), coordinated_shutdown, spawn_forwarding_task
  nullslop-services/   # Services container, ActorChannelService, CoreChannelService
  nullslop-tui/        # Terminal, renderer, keymap (produces Intent), event loop
  nullslop-actor-host/   # Actor host implementations
  nullslop-actor/        # Actor author SDK
  nullslop-cli/        # CLI argument parsing
actors/
  nullslop-coordinator/       # Coordinator actor
  nullslop-projector/         # Projector actor
  nullslop-shutdown-tracker/  # ShutdownTracker actor
  nullslop-llm/               # LLM provider actor
  nullslop-session-actor/     # Session persistence actor
  nullslop-context-actor/     # Context assembly actor
  nullslop-tool-orchestrator/ # Tool execution actor
  nullslop-echo/              # Example echo actor
  nullslop-prompt-scan/       # Prompt template scanning actor
  nullslop-llm-discover/      # LLM discovery actor
```

**Component module pattern (under `nullslop-component/src/`):**

```
chat_input_box/
├── mod.rs      # Re-exports and public interface
├── element.rs  # UiElement<AppState> rendering
└── state.rs    # Component-specific state (e.g., ChatInputBoxState)
```

Not every component needs all three files. A display-only component (like chat log) may only have `mod.rs` and `element.rs`.

### Dependency Injection

**Services container (in `nullslop-services`):**

```rust
#[derive(Debug, Clone)]
pub struct Services {
    pub handle: Handle,
    pub actor_channel: ActorChannelService,
    pub core_channel: CoreChannelService,
    pub llm_service: LlmServiceFactoryService,
    pub provider_registry: ProviderRegistryService,
    pub api_keys: ApiKeysService,
    pub config_storage: ConfigStorageService,
    pub session_store: SessionStoreService,
    pub strategy_registry: StrategyRegistryService,
}
```

Created once at startup and shared throughout the application. `Services` is the DI container for the actor system — the frontend and intent handler don't need it because they work with `AppState` directly.

All services within the `Services` struct must either:

- Be cheap to clone
- Use the "service wrapper" pattern detailed above.

## 3. Data Flow

See [ARCHITECTURE.md](./ARCHITECTURE.md) for the full data flow diagram. The flow is unidirectional: user input → IntentHandler → commands → actor system (coordinator, actors) → events → projector → AppState → renderer.

## 4. Tests

Important:

- Tests should only verify _observable behavior_
- Testing internal details is an _anti-pattern_.
- Prefer testing observable behavior ONLY. If observable behavior cannot be tested, then an abstraction needs to be created. Ask the user how to proceed in this case.

### One Test, One Behavior

**Every test must assert exactly one semantic concept.** A test should answer a single question about the system. When it fails, the test name alone must tell you _what_ broke.

This means each test has exactly **one** `// When` and **one** `// Then` block. A `// Then` may be followed by `// And` lines, but only those lines elaborate on the same observable behavior — never when they describe a different behavior.

**What counts as "one concept":**

- Checking multiple fields of the _same result_ — fine. All confirms "the result is correct."
- Checking that a serialization roundtrip preserved all fields — fine. All confirms "the roundtrip worked."
- Checking that reset cleared all fields — fine. All confirms "everything was reset."
- Checking that every item in a filtered list matches the filter — fine. All confirms "the filter worked."

**What counts as separate concepts (split into separate tests):**

- A handler that updates state **and** emits a command → two tests. State change and command emission are separate observable behaviors.
- Processing a second input after a first → two tests. Each input triggers its own behavior.
- Rendering multiple entry types from one widget → one test per entry type. Each entry type is a separate rendering behavior.
- A multi-step lifecycle (start, complete, finalize, advance) → one test per step. Each step is a separate state transition.

**Anti-patterns to avoid:**

```rust
// ❌ BAD — two When/Then blocks in one test
#[test]
fn stream_token_appends_to_assistant_entry() {
    // ...setup...
    // When processing the first token.
    // Then the entry has "Hello".
    // When processing a second token.
    // Then the text is "Hello world".
}
```

```rust
// ✅ GOOD — split into two tests
#[test]
fn first_stream_token_creates_assistant_entry() {
    // ...setup...
    // When processing StreamToken("Hello").
    // Then the session has an Assistant entry with "Hello".
}

#[test]
fn subsequent_stream_token_appends_to_existing_entry() {
    // ...setup with one token already processed...
    // When processing another StreamToken(" world").
    // Then the text is "Hello world".
}
```

```rust
// ❌ BAD — checking state change AND command emission in one test
#[test]
fn submit_message_clears_input_and_enqueues() {
    // ...setup...
    // When submitting a message.
    // Then the input buffer is cleared.
    // And EnqueueUserMessage was returned.
}
```

```rust
// ✅ GOOD — split into separate tests
#[test]
fn submit_message_clears_input_buffer() {
    // ...setup...
    // When handling Intent::SubmitMessage.
    // Then the input buffer is empty.
}

#[test]
fn submit_message_returns_enqueue_command() {
    // ...setup...
    // When handling Intent::SubmitMessage.
    // Then the result contains EnqueueUserMessage.
}
```

```rust
// ❌ BAD — checking multiple entry type renders in one test
#[test]
fn render_mixed_entries() {
    // Given system, user, actor, and assistant entries.
    // When rendering.
    // Then line 6 is system (dark gray).
    // And line 7 is user (">" prefix, bold).
    // And line 8 is actor (yellow).
    // And line 9 is assistant (cyan).
}
```

```rust
// ✅ GOOD — one test per entry type
#[test]
fn render_system_entry_is_dark_gray() {
    // Given a ChatLogElement with a system entry.
    // When rendering.
    // Then the system entry line has dark gray foreground.
}

#[test]
fn render_user_entry_has_prefix() {
    // Given a ChatLogElement with a user entry.
    // When rendering.
    // Then the user entry line starts with ">".
}
```

**Duplicated test setup is acceptable.** Do not combine tests to avoid setup duplication.

### BDD-Style Tests (Given/When/Then)

Structure tests with clear Given/When/Then sections, and name the test so it can be read as a standalone program behavior in the test report:

```rust
fn pop_returns_none_when_stack_empty() {
    // Given an empty stack.
    let mut stack = Stack::default();

    // When popping from the stack.
    let item = stack.pop();

    // Then we get nothing back.
    assert!(item.is_none());
}
```

**Example — testing the intent handler:**

```rust
#[test]
fn quit_sets_should_quit_in_state() {
    // Given default app state.
    let mut state = AppState::default();

    // When handling Intent::Quit.
    let result = IntentHandler::handle(&Intent::Quit, &mut state);

    // Then should_quit is set to true.
    assert!(state.should_quit);
    // And no commands are emitted.
    assert!(result.commands.is_empty());
}
```

**Example — testing a validator:**

```rust
#[test]
fn submit_message_rejected_when_buffer_empty() {
    // Given an empty input buffer.
    let state = AppState::default();

    // When validating submit message.
    let result = validate_submit_message(&state);

    // Then validation fails with EmptyBuffer.
    assert!(matches!(result, Err(SubmitMessageError::EmptyBuffer)));
}
```

**Example — testing a projector:**

```rust
#[test]
fn stream_token_appends_to_assistant_entry() {
    // Given a projector with an active session.
    let state = State::new(AppState::default());
    let sink = RecordingSink::new();
    let projector = ProjectorActor::new(state.clone());

    // When handling StreamToken("Hello").
    projector.on_stream_token(&StreamToken { /* ... */ }, &sink);

    // Then the session has an Assistant entry with "Hello".
    let s = state.read();
    assert_eq!(s.active_session().last_entry_text(), "Hello");
}
```

### Parameterized Tests with rstest

If a test has many inputs, prefer parametrizing with `rstest`:

```rust
#[rstest::rstest]
#[case(Key::Tab, "Tab")]
#[case(Key::Enter, "Enter")]
fn key_display(#[case] key: Key, #[case] expected: &str) {
    // Given / When / Then inline for simple cases
    assert_eq!(key.display(), expected);
}
```

For edge cases that don't easily fit into "expected", prefer a BDD-styled test instead.

Use rstest when you find yourself writing the same assertion logic against different inputs. Do _not_ use rstest to combine different behaviors into one test — each `#[case]` must test the same property.

### Async Tests

```rust
#[tokio::test]
async fn actor_host_loads_manifest() {
    // Given an in-memory actor host.
    let host = InMemoryActorHost::new();

    // When loading actors.
    let result = host.discover().await;

    // Then discovery succeeds.
    assert!(result.is_ok());
}
```

### Test Utilities

**`test_utils` module structure:**

```rust
// test_utils/mod.rs
pub mod context;
pub mod fakes;
pub mod fixtures;
pub mod services;
```

**Shared test helpers:**

- `RecordingSink` (in `nullslop-actor`) — records messages emitted by actors during tests. Shared across all actor crates.
- `TuiApp` test builder — simplified construction of `TuiApp` for render and app tests.
- `setup_term(width, height)` — creates a ratatui `TestBackend` terminal with the given dimensions.
- `ChatSessionState` test builder — simplifies lifecycle setup for session-dependent tests.

## 5. Documentation

### Module-Level Documentation

Module level documentation should explain its purpose and high-level behaviors. Only explain technical details as necessary to make the high-level documentation understandable.

```rust
//! Chat input box — where the user composes and sends messages.
//!
//! This component manages the text input experience end to end: handling keystrokes,
//! displaying the in-progress message, and switching between browsing and typing modes.
```

### Type Documentation

```rust
/// The user's in-progress message being composed in the input box.
#[derive(Debug)]
pub struct ChatInputBoxState {
    /// The text the user has typed so far.
    input_buffer: String,
}
```

## 6. Modification Guide

When implementing features:

1. **Add Intent variant** — in `nullslop-intent/src/intent.rs` (or wherever the `Intent` enum is defined in `nullslop-protocol/src/intent.rs`)
2. **Add validator** — in `nullslop-intent/src/validators/` as a dedicated function. Infallible intents don't need a validator (removed in Phase 10.7). Fallible ones return `Result<(), SpecificError>`.
3. **Add handler match arm** — in `nullslop-intent/src/handler.rs`: call validator (if any), mutate `AppState`, return commands
4. **Add keymap binding** — in `nullslop-tui/src/keymap.rs`: bind key to `Intent` variant in the right scope (`Normal`, `Input`, `Dashboard`, `Pinned`, `Picker`)
5. **Add Command/Event if needed** — define domain-only structs in `nullslop-protocol` with corresponding `Command` or `Event` enum variants. Forgetting the enum variant is the most common oversight — the struct alone is not enough.
6. **Add coordinator logic if needed** — subscribe to the new command in `actors/nullslop-coordinator` and implement the handler
7. **Add projector handler if needed** — subscribe to the new event in `actors/nullslop-projector` and implement the state mutation
8. **Add UI element if needed** — in `nullslop-component/src/`, register in `register_all()` in `lib.rs`
9. **Write tests** — Use Given/When/Then structure: test validator in isolation, test intent handler for state changes and commands, test projector for event→state mapping
10. **Add documentation** — Module docs, type docs, error docs. Describe behavior and purpose, not technical implementation.

## 8. Tooling

Read the `justfile` to determine what additional tooling is related to this project. Prioritize running commands from the `justfile` instead of manual invocation. If there is a `just test` command, then use that instead of `cargo test`, etc.

## 9. Misc

- NEVER manually split a string using `.chars` or by indexing. Use the `unicode-segmentation` crate.
- No trivial setters for struct methods. Prefer meaningful semantic actions. It's an anti-pattern to directly inspect and manipulate state.
- Environment variables should only be accessed at program initialization and then saved into a struct as needed. Environment variables are a global namespace and should be avoided outside of program startup.
- Use `where` clause for all generics.
