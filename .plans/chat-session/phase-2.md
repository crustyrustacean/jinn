# Phase 2: Wire up the new crate and update consumers

## Problem

The new `nsslice-chat-session-protocol` crate exists with all types and tests, but no consumers use it yet. We need to:
1. Add it as a dependency of `nullslop-component` and re-export `ChatSessionState`
2. Update all downstream consumers to import from the new crate

## What Moves / What Stays

**Changes (import paths only, no logic changes):**
- `nullslop-component/src/lib.rs` — replace `pub mod chat_session` with re-export from new crate
- `nullslop-component/src/app_state.rs` — change import path
- `nullslop-component/Cargo.toml` — add dependency
- `nsslice-session-management/Cargo.toml` — add dependency
- `nsslice-session-management/src/intent.rs` — change import path
- Root `Cargo.toml` — add dependency
- `src/session_conversion.rs` — change import path

**No changes to:**
- Any `ChatSessionState` logic
- Any tests (tests remain in both old location and new crate for now)
- Any other crates

## File Changes

### 1. MODIFY `crates/common/nullslop-component/Cargo.toml`

Add `nsslice-chat-session-protocol = { workspace = true }` to `[dependencies]`.

### 2. MODIFY `crates/common/nullslop-component/src/lib.rs`

Replace:
```rust
pub mod chat_session;
```
With nothing (remove the module).

Replace:
```rust
pub use chat_session::ChatSessionState;
```
With:
```rust
pub use nsslice_chat_session_protocol::ChatSessionState;
```

### 3. MODIFY `crates/common/nullslop-component/src/app_state.rs`

Replace:
```rust
use crate::chat_session::ChatSessionState;
```
With:
```rust
use nsslice_chat_session_protocol::ChatSessionState;
```

### 4. MODIFY `crates/slices/nsslice-session-management/Cargo.toml`

Add `nsslice-chat-session-protocol = { workspace = true }` to `[dependencies]`.

### 5. MODIFY `crates/slices/nsslice-session-management/src/intent.rs`

Replace:
```rust
nullslop_component::chat_session::ChatSessionState::new(),
```
With:
```rust
nsslice_chat_session_protocol::ChatSessionState::new(),
```

### 6. MODIFY root `Cargo.toml`

Add `nsslice-chat-session-protocol = { workspace = true }` to `[dependencies]`.

### 7. MODIFY `src/session_conversion.rs`

Replace:
```rust
use nullslop_component::chat_session::ChatSessionState;
```
With:
```rust
use nsslice_chat_session_protocol::ChatSessionState;
```

And in the test module:
```rust
use nullslop_component::chat_session::ChatSessionState;
```
With:
```rust
use nsslice_chat_session_protocol::ChatSessionState;
```

## Implementation Order

1. Update `nullslop-component/Cargo.toml` — add dependency
2. Update `nullslop-component/src/lib.rs` — re-export from new crate
3. Update `nullslop-component/src/app_state.rs` — change import
4. Update `nsslice-session-management/Cargo.toml` — add dependency
5. Update `nsslice-session-management/src/intent.rs` — change import
6. Update root `Cargo.toml` — add dependency
7. Update `src/session_conversion.rs` — change imports
8. Run `just check`

## Acceptance Criteria

- [x] `nullslop-component` depends on `nsslice-chat-session-protocol` and re-exports `ChatSessionState`
- [x] `nullslop-component/src/app_state.rs` imports `ChatSessionState` from `nsslice_chat_session_protocol`
- [x] `nsslice-session-management` imports `ChatSessionState` from `nsslice_chat_session_protocol`
- [x] `src/session_conversion.rs` imports `ChatSessionState` from `nsslice_chat_session_protocol`
- [x] `just check` passes (full workspace compiles)
