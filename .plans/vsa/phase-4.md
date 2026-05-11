# Phase 4: Introduce Intent Registration Convention

This phase moves `IntentResult` from `nullslop-intent` to `nullslop-protocol` so that
future intent-bearing slice crates (Phases 5–6) can return `IntentResult` without
depending on `nullslop-intent`. It also documents the intent-bearing slice convention.

This is a **no-behavior-change** phase — pure type relocation + documentation.

## Context

`IntentResult` is currently defined in `nullslop-intent/src/handler.rs`:

```rust
#[derive(Debug)]
pub struct IntentResult {
    pub commands: Vec<Command>,
}

impl IntentResult {
    pub fn empty() -> Self { ... }
    pub fn with_commands(commands: Vec<Command>) -> Self { ... }
}
```

It depends only on `Command`, which is already in `nullslop-protocol`. No external
crate imports `IntentResult` — it's used exclusively within `nullslop-intent` (in
`handler.rs` and `handler_tests.rs`). The re-export from `nullslop-intent/src/lib.rs`
via `pub use handler::IntentResult` exists but has no external consumers today.

Moving it to `nullslop-protocol` makes it available to slice crates that need to
return `IntentResult` from their handler functions (e.g., `nsslice-pinned-panel` in
Phase 5). The slice crates already depend on `nullslop-protocol` for `Command`,
`Intent`, `PinPosition`, etc.

## Detailed Steps

### 1. Create `nullslop-protocol/src/intent_result.rs`

New file containing the `IntentResult` struct, moved verbatim from `handler.rs`:

```rust
//! Result type returned by intent handlers.
//!
//! Carries commands to be dispatched to the actor system.
//! Lives in the protocol crate so that slice crates can return it
//! without depending on `nullslop-intent`.

use crate::Command;

/// What an intent handler returns after processing an intent.
#[derive(Debug)]
pub struct IntentResult {
    /// Commands to send to the actor system.
    pub commands: Vec<Command>,
}

impl IntentResult {
    /// An empty result with no commands.
    #[must_use]
    pub fn empty() -> Self {
        Self { commands: vec![] }
    }

    /// A result with commands.
    #[must_use]
    pub fn with_commands(commands: Vec<Command>) -> Self {
        Self { commands }
    }
}
```

### 2. Update `nullslop-protocol/src/lib.rs`

Add `pub mod intent_result;` to the module declarations and `pub use intent_result::IntentResult;` to the re-exports.

### 3. Update `nullslop-intent/src/handler.rs`

- Remove the `IntentResult` struct definition and its `impl` block entirely
- Add `use nullslop_protocol::IntentResult;` to imports (or use the re-export via `crate` — see step 4)

### 4. Update `nullslop-intent/src/lib.rs`

Replace the current re-export:
```rust
pub use handler::{IntentHandler, IntentResult};
```
With:
```rust
pub use handler::IntentHandler;
pub use nullslop_protocol::IntentResult;
```

This preserves backward compatibility — `use nullslop_intent::IntentResult` still works
for any future consumer, while the actual type lives in `nullslop-protocol`.

### 5. Update `nullslop-intent/src/handler_tests.rs`

The test helper `fn handle(...)` returns `super::IntentResult`. Since `IntentResult`
is now re-exported via `crate::IntentResult` (from `lib.rs`), `super::IntentResult`
in the `tests` module of `handler.rs` still resolves correctly through the `use nullslop_protocol::IntentResult` import added in `handler.rs`.

No changes needed in `handler_tests.rs`.

### 6. Document the intent-bearing slice convention

Add a documentation comment to `nsslice-chat-log/src/lib.rs` (the most recently
created display-only slice, serving as the "base case"). This documents what
an intent-bearing slice will look like, so the Phase 5 implementer has a reference:

```rust
//! Chat log slice — renders the full conversation history.
//!
//! A display-only component showing all messages exchanged in the active session.
//! ...
//!
//! # Slice Convention
//!
//! Display-only slices (like this one) contain only an element module.
//! Intent-bearing slices extend this pattern:
//!
//! ```text
//! nsslice-<feature>/
//! ├── src/
//! │   ├── lib.rs        — register() + re-exports
//! │   ├── element.rs    — UiElement impl + tests
//! │   ├── intent.rs     — pub fn handle_<intent>(state: &mut AppState) -> IntentResult
//! │   └── validator.rs  — pub fn validate_<intent>(state: &AppState) -> Result<(), Error> + tests
//! ```
//!
//! The central `IntentHandler::handle()` match block in `nullslop-intent`
//! calls into slice handler functions. `IntentResult` lives in `nullslop-protocol`.
```

### 7. Verify

```bash
cargo test --workspace
cargo test -p nullslop-intent
cargo test -p nullslop-protocol
```

## Acceptance Criteria

1. **`IntentResult` defined in `nullslop-protocol`**: The struct and its methods live in `nullslop-protocol/src/intent_result.rs`
2. **No duplicate definitions**: `IntentResult` is not defined in `nullslop-intent/src/handler.rs`
3. **Backward-compatible re-export**: `nullslop_intent::IntentResult` still compiles (re-exported from `nullslop-protocol`)
4. **No external breakage**: All workspace tests pass (`cargo test --workspace`)
5. **Convention documented**: `nsslice-chat-log/src/lib.rs` has doc comments explaining the intent-bearing slice pattern
6. **No new dependencies**: `nullslop-protocol` does not gain new crate dependencies (it already has `Command`)

## Files Changed

### Created
- `crates/nullslop-protocol/src/intent_result.rs`

### Modified
- `crates/nullslop-protocol/src/lib.rs` — add `pub mod intent_result;` + re-export
- `crates/nullslop-intent/src/handler.rs` — remove `IntentResult` definition, add `use nullslop_protocol::IntentResult;`
- `crates/nullslop-intent/src/lib.rs` — change re-export to `pub use nullslop_protocol::IntentResult;`
- `crates/slices/nsslice-chat-log/src/lib.rs` — add convention documentation
