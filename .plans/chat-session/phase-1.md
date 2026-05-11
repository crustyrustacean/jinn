# Phase 1: Create nsslice-chat-session-protocol crate

## Problem

`ChatSessionState` (plus `SessionCore`, `SessionUi`, and ~500 lines of tests) lives inside `nullslop-component`. We need a new standalone protocol crate that owns these types, following the same pattern as `nsslice-chat-input-box-protocol`.

## What Moves / What Stays

**Moves into `nsslice-chat-session-protocol`:**
- `ChatSessionState` struct + all methods (streaming, queue, sending, assembling, scroll, selection, pinning, strategy, history, tool calls)
- `SessionCore` struct + `Default` impl
- `SessionUi` struct + `Default` impl
- `ChatSessionStateBuilder` + `BuilderOp` enum (`#[cfg(test)]`)
- All tests from `state_tests.rs`

**Stays in `nullslop-component` (unchanged):**
- `AppState`, `SessionState`, `FrontendState`, etc. in `app_state.rs`
- `State`, `StateReadGuard`, `StateWriteGuard` in `state.rs`
- `PromptTemplateStore` in `prompt_template/`
- `TuiSignals` in `tui_signals.rs`
- `lib.rs` module structure (will be updated in Phase 2)

## File Changes

### 1. CREATE `crates/slices/nsslice-chat-session-protocol/Cargo.toml`

```toml
[package]
name = "nsslice-chat-session-protocol"
version = "0.1.0"
edition = "2024"

[dependencies]
nullslop-protocol = { workspace = true }
nsslice-chat-input-box-protocol = { workspace = true }
serde_json = { workspace = true }

[dev-dependencies]
rstest = { workspace = true }

[lints]
workspace = true
```

### 2. CREATE `crates/slices/nsslice-chat-session-protocol/src/lib.rs`

Copy the entire content of `nullslop-component/src/chat_session/state.rs` with these import changes:

- Remove `use nsslice_chat_input_box_protocol::ChatInputBoxState;` (it's already imported via the module, but in the new crate it needs the same import)
- Keep all other imports as-is — they reference `nullslop_protocol` types
- Add module doc at crate level
- Add `#[cfg(test)] mod tests;` at the bottom (instead of `#[cfg(test)] #[path = "state_tests.rs"] mod state_tests;`)

The module structure:
```rust
//! Chat session protocol — state types for a single conversation.
//!
//! ...

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU16, Ordering};

use nullslop_protocol::{ChatEntry, ChatEntryId, ChatEntryKind, PinPosition};
use serde_json::Value as JsonValue;

use nsslice_chat_input_box_protocol::ChatInputBoxState;

// ... SessionCore, SessionUi, ChatSessionState, impl blocks ...

#[cfg(test)]
impl ChatSessionState {
    pub fn builder() -> ChatSessionStateBuilder { ... }
}

#[cfg(test)]
#[derive(Debug, Default)]
pub struct ChatSessionStateBuilder { ... }

#[cfg(test)]
#[derive(Debug)]
enum BuilderOp { ... }

#[cfg(test)]
impl ChatSessionStateBuilder { ... }

#[cfg(test)]
mod tests;
```

### 3. CREATE `crates/slices/nsslice-chat-session-protocol/src/tests.rs`

Copy the entire content of `nullslop-component/src/chat_session/state_tests.rs` with updated imports:

```rust
use nullslop_protocol::{ChatEntry, ChatEntryId, ChatEntryKind, PinPosition};

use super::*;
```

The test file already imports `ChatEntry`, `ChatEntryId`, `ChatEntryKind`, `PinPosition` from `nullslop_protocol` — these stay the same. The `use super::*;` will now resolve to the new crate's types.

### 4. MODIFY workspace `Cargo.toml`

Add to `[workspace.dependencies]`:
```toml
nsslice-chat-session-protocol = { path = "crates/slices/nsslice-chat-session-protocol" }
```

The `members = ["crates/common/*", "crates/slices/*"]` glob already covers the new directory.

## Implementation Order

1. Create `Cargo.toml` for the new crate
2. Create `src/lib.rs` — copy `state.rs` content, update module-level docs and imports
3. Create `src/tests.rs` — copy `state_tests.rs` content
4. Add workspace dependency entry
5. Run `just check` to verify the new crate compiles in isolation
6. Run `cargo test -p nsslice-chat-session-protocol` to verify tests pass

## Acceptance Criteria

- [x] `crates/slices/nsslice-chat-session-protocol/Cargo.toml` exists with correct dependencies
- [x] `crates/slices/nsslice-chat-session-protocol/src/lib.rs` exists and contains `ChatSessionState`, `SessionCore`, `SessionUi`
- [x] `crates/slices/nsslice-chat-session-protocol/src/tests.rs` exists with all tests moved
- [x] `nsslice-chat-session-protocol` is listed in workspace `[workspace.dependencies]`
- [x] `cargo check -p nsslice-chat-session-protocol` succeeds
- [x] `cargo test -p nsslice-chat-session-protocol` passes (70/70 tests pass)
- [x] Original `nullslop-component` still compiles (`cargo check -p nullslop-component` succeeds — nothing deleted yet)
