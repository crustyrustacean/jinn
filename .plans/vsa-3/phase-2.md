# Phase 2: Extract Session & Model → `nsslice-session-management`

## Problem

Three session/model intents (`SessionNew`, `RefreshModels`, `RescanPromptTemplates`) are handled by free functions in `nullslop-intent/src/handler.rs` with validators in `nullslop-intent/src/validators/chat_entry.rs`. These move to a dedicated slice crate, and the entire `chat_entry.rs` validator file is deleted.

## What Moves

### From `nullslop-intent/src/validators/chat_entry.rs` (entire file → deleted):

**3 error types + 3 validator functions + 4 tests:**

1. `RefreshModelsError` enum + `validate_refresh_models(state)` — fallible, checks `active_provider != NO_PROVIDER_ID`
2. `RescanPromptTemplatesError` enum + `validate_rescan_prompt_templates(state)` — semi-fallible (always succeeds currently)
3. `SessionNewError` enum + `validate_session_new(state)` — fallible, checks no picker active

**4 tests:**
- `refresh_models_succeeds_with_provider`
- `refresh_models_fails_with_no_provider`
- `session_new_succeeds_when_no_picker_active`
- `session_new_fails_when_picker_active`

### From `nullslop-intent/src/handler.rs` (3 free functions → removed):

1. `handle_session_new(state)` — validates, removes old session, creates new one, sets mode Normal
2. `handle_refresh_models(state)` — validates, posts system message, returns `Command::RefreshModels`
3. `handle_rescan_prompt_templates(state)` — validates (ignores result), posts system message, returns `Command::RescanPromptTemplates`

### From `nullslop-intent/src/handler_tests.rs` (5 tests → removed):

1. `session_new_creates_fresh_session`
2. `session_new_noop_when_picker_active`
3. `refresh_models_posts_system_message_and_returns_command`
4. `refresh_models_noop_with_no_provider`
5. `rescan_prompt_templates_posts_system_message_and_returns_command`

### What stays in `nullslop-intent`

Everything else. No changes to other match arms, helper functions, or the `app` validator module. The `validators/mod.rs` loses `pub mod chat_entry;`.

After this phase, `handler.rs` will have `chat_entry` removed from imports and the `SessionNew`/`RefreshModels`/`RescanPromptTemplates` match arms will delegate to the new crate.

## File Changes

### 1. NEW `crates/slices/nsslice-session-management/Cargo.toml`

```toml
[package]
name = "nsslice-session-management"
version = "0.1.0"
edition = "2024"

[dependencies]
nullslop-component = { workspace = true }
nullslop-protocol = { workspace = true }
wherror = { workspace = true }

[dev-dependencies]
rstest = { workspace = true }

[lints]
workspace = true
```

- `nullslop-component` — for `AppState`, `ChatSessionState`, `NO_PROVIDER_ID`
- `nullslop-protocol` — for `IntentResult`, `Command`, `Mode`, `ChatEntry`
- `wherror` — for `#[derive(Error)]` on validator error types
- `rstest` — test framework (dev-dep)

### 2. NEW `crates/slices/nsslice-session-management/src/lib.rs`

```rust
//! Session management slice — session creation, model refresh, and prompt template rescan.
//!
//! Handles creating new sessions, refreshing the model list from the
//! active provider, and rescanning prompt templates. No element —
//! rendering stays in `nullslop-tui`.

pub mod intent;
pub mod validator;
```

No element — no `register()`.

### 3. NEW `crates/slices/nsslice-session-management/src/validator.rs`

Contains the entire contents of `nullslop-intent/src/validators/chat_entry.rs` (3 error types, 3 validator functions, 4 tests). Module doc comment updates to reflect new location.

**Imports:**
```rust
use nullslop_component::AppState;
use nullslop_protocol::PickerKind;
use wherror::Error;
```

The `PickerKind` import is needed by the `session_new_fails_when_picker_active` test. The `nullslop_protocol` re-export is used via `nullslop_protocol::PickerKind::Provider` in the test body.

### 4. NEW `crates/slices/nsslice-session-management/src/intent.rs`

Contains 3 public handler functions + 5 tests.

**Imports:**
```rust
use nullslop_component::AppState;
use nullslop_protocol::{ChatEntry, Command, IntentResult, Mode, SessionId};

use crate::validator;
```

**Handler functions:**

```rust
/// Creates a new chat session, replacing the active one.
pub fn handle_session_new(state: &mut AppState) -> IntentResult {
    if validator::validate_session_new(state).is_err() {
        return IntentResult::empty();
    }

    state.session.sessions.remove(&state.session.active_session);

    let new_id = SessionId::new();
    state.session.sessions.insert(
        new_id.clone(),
        nullslop_component::chat_session::ChatSessionState::new(),
    );
    state.session.active_session = new_id;
    state.frontend.mode = Mode::Normal;

    IntentResult::empty()
}

/// Refreshes the model list from the active provider.
pub fn handle_refresh_models(state: &mut AppState) -> IntentResult {
    if validator::validate_refresh_models(state).is_err() {
        return IntentResult::empty();
    }

    state
        .active_session_mut()
        .push_entry(ChatEntry::system("Refreshing models..."));

    IntentResult::with_commands(vec![Command::RefreshModels])
}

/// Rescans prompt templates from disk.
pub fn handle_rescan_prompt_templates(state: &mut AppState) -> IntentResult {
    let _ = validator::validate_rescan_prompt_templates(state);

    state
        .active_session_mut()
        .push_entry(ChatEntry::system("Rescanning prompt templates..."));

    IntentResult::with_commands(vec![Command::RescanPromptTemplates])
}
```

**5 tests** (moved from `handler_tests.rs`). In the new crate, tests call handler functions directly. Test imports:

```rust
#[cfg(test)]
mod tests {
    use nullslop_component::{AppState, FrontendState, ProviderState};
    use nullslop_protocol::{ChatEntry, Command, Mode, PickerKind};

    use super::*;

    // ... 5 tests
}
```

### 5. Root `Cargo.toml`

**members line** — add `"crates/slices/nsslice-session-management"`:

```
members = [..., "crates/slices/nsslice-navigation", "crates/slices/nsslice-session-management", "actors/*", "tests/*"]
```

**[workspace.dependencies]** — add:
```toml
nsslice-session-management = { path = "crates/slices/nsslice-session-management" }
```

**[dependencies]** — add:
```toml
nsslice-session-management = { workspace = true }
```

### 6. `crates/nullslop-intent/Cargo.toml`

Add:
```toml
nsslice-session-management = { workspace = true }
```

### 7. `crates/nullslop-intent/src/handler.rs`

**Replace 3 session/model match arms** with delegations:

```rust
// Old:
Intent::SessionNew => handle_session_new(state),
Intent::RefreshModels => handle_refresh_models(state),
Intent::RescanPromptTemplates => handle_rescan_prompt_templates(state),

// New:
Intent::SessionNew => {
    nsslice_session_management::intent::handle_session_new(state)
}
Intent::RefreshModels => {
    nsslice_session_management::intent::handle_refresh_models(state)
}
Intent::RescanPromptTemplates => {
    nsslice_session_management::intent::handle_rescan_prompt_templates(state)
}
```

**Remove 3 free functions**: `handle_session_new`, `handle_refresh_models`, `handle_rescan_prompt_templates`.

**Clean up imports**: Remove `chat_entry` from the validator import line:

```rust
// Old:
use crate::validators::{app, chat_entry};

// New:
use crate::validators::app;
```

Also remove `SessionId` and `Command` from the protocol import line — check if still needed:

- `SessionId` — only used in `handle_session_new` (removed) and `handle_interrupt` (still uses it via `state.session.active_session.clone()`). **Keep `SessionId`** — wait, actually `SessionId` is used in `handle_session_new` (removing) and `handle_interrupt` constructs `CancelStream { session_id }`. Let me check... `handle_interrupt` uses `state.session.active_session.clone()` which returns a `SessionId`. The `CancelStream` struct likely has a `SessionId` field. Looking at the code: `Command::CancelStream { payload: CancelStream { session_id } }` — `session_id` is of type `SessionId`. So `SessionId` is still needed by `handle_interrupt`. **Keep.**
- `Command` — used in `handle_interrupt`, `handle_set_mode`. **Keep.**
- `Mode` — used in `handle_set_mode`, `handle_normal_escape`. **Keep.**
- `CancelStream` — used in `handle_interrupt`, `handle_set_mode`. **Keep.**
- `PinPosition` — used in PinnedPanel match arms. **Keep.**

So the only import change in `handler.rs` is:
```rust
// Old:
use crate::validators::{app, chat_entry};
// New:
use crate::validators::app;
```

### 8. `crates/nullslop-intent/src/handler_tests.rs`

**Remove 5 tests:**

- `session_new_creates_fresh_session`
- `session_new_noop_when_picker_active`
- `refresh_models_posts_system_message_and_returns_command`
- `refresh_models_noop_with_no_provider`
- `rescan_prompt_templates_posts_system_message_and_returns_command`

**Also remove the section headers** that become empty:
- `// ============================================================\n// SessionNew Intent\n// ============================================================`
- `// ============================================================\n// RefreshModels & RescanPromptTemplates\n// ============================================================`

**Clean up imports**: Remove `ProviderState` (only used by `refresh_models_posts_system_message_and_returns_command`):

```rust
// Old:
use nullslop_component::{AppState, FrontendState, ProviderState};
// New:
use nullslop_component::{AppState, FrontendState};
```

All other imports (`ChatEntry`, `Command`, `Mode`, `PickerKind`, `KeymapEntry`) remain used by the remaining tests.

### 9. Delete `crates/nullslop-intent/src/validators/chat_entry.rs`

The entire file moves to `nsslice-session-management/src/validator.rs`.

### 10. Update `crates/nullslop-intent/src/validators/mod.rs`

Remove `pub mod chat_entry;`:

```rust
// Old:
pub mod app;
pub mod chat_entry;

// New:
pub mod app;
```

Update the module doc comment to remove mention of chat_entry (currently says nothing specific about it, so no change needed — just remove the module line).

## Implementation Order

1. Create `crates/slices/nsslice-session-management/Cargo.toml`
2. Create `crates/slices/nsslice-session-management/src/lib.rs`
3. Create `crates/slices/nsslice-session-management/src/validator.rs` — 3 error types + 3 validators + 4 tests
4. Create `crates/slices/nsslice-session-management/src/intent.rs` — 3 handlers + 5 tests
5. Add `nsslice-session-management` to root `Cargo.toml` (members, workspace.dependencies, dependencies)
6. Add `nsslice-session-management` dep to `nullslop-intent/Cargo.toml`
7. Update `nullslop-intent/src/handler.rs` — replace 3 match arms, remove 3 free functions, clean imports
8. Update `nullslop-intent/src/handler_tests.rs` — remove 5 tests, clean imports
9. Delete `nullslop-intent/src/validators/chat_entry.rs`
10. Update `nullslop-intent/src/validators/mod.rs` — remove `pub mod chat_entry;`
11. Run `cargo test --workspace`

## Acceptance Criteria

1. `crates/slices/nsslice-session-management/` exists with `Cargo.toml`, `src/lib.rs`, `src/validator.rs`, `src/intent.rs`
2. `nsslice-session-management` is a workspace member in root `Cargo.toml`
3. `nullslop-intent/Cargo.toml` has `nsslice-session-management` dependency
4. `nsslice-session-management` has 4 validator tests + 5 handler tests (9 total), all passing independently (`cargo test -p nsslice-session-management`)
5. `nullslop-intent/src/handler.rs` has 3 session/model match arms delegating to `nsslice_session_management::intent::*`
6. `nullslop-intent/src/handler.rs` no longer has `handle_session_new`, `handle_refresh_models`, `handle_rescan_prompt_templates`, or `chat_entry` import
7. `nullslop-intent/src/handler_tests.rs` no longer has session/model tests or `ProviderState` import
8. `nullslop-intent/src/validators/chat_entry.rs` no longer exists
9. `nullslop-intent/src/validators/mod.rs` no longer has `pub mod chat_entry;`
10. `cargo test --workspace` passes — no regressions
