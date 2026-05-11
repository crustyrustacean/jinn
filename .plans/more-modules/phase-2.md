# Phase 2: Extract inline code from `session/mod.rs` and convert

## Problem

`session/mod.rs` contains inline code: the `SessionId` struct, its impls, and a `#[cfg(test)] mod tests` block. To convert it to `session.rs` (no `mod.rs`), we must first extract the inline code into a dedicated file, making the module root purely declarative.

## What Moves

- `SessionId` struct, `impl SessionId`, `impl Default for SessionId`, and the `#[cfg(test)] mod tests` block move from `session/mod.rs` → `session/session_id.rs`
- `session/mod.rs` is then rewritten to be purely declarative and moved to `session.rs`

## File Changes

### 1. Create `session/session_id.rs` (new file)

```rust
//! Unique identifier for a chat session.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A unique identifier for a chat session.
///
/// Generated using UUID v4, stored as an opaque string.
/// Derives equality and hashing so it can be used as a `HashMap` key.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(String);

impl SessionId {
    /// Generate a new unique session ID using UUID v4.
    #[must_use]
    pub fn new() -> Self {
        Self(format!("s-{}", Uuid::new_v4()))
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[rstest::rstest]
    fn session_id_new_generates_unique_ids() {
        // Given nothing.
        // When generating two session IDs.
        let a = SessionId::new();
        let b = SessionId::new();

        // Then they are different.
        assert_ne!(a, b);
    }

    #[rstest::rstest]
    fn session_id_serialization_roundtrip() {
        // Given a session ID.
        let id = SessionId::new();

        // When serializing and deserializing.
        let json = serde_json::to_string(&id).expect("serialize");
        let back: SessionId = serde_json::from_str(&json).expect("deserialize");

        // Then it roundtrips correctly.
        assert_eq!(id, back);
    }

    #[rstest::rstest]
    fn session_id_starts_with_prefix() {
        // Given a new session ID.
        let id = SessionId::new();

        // When inspecting the string representation.
        // Note: we can't access the inner String directly, so we check serialization.
        let json = serde_json::to_string(&id).expect("serialize");

        // Then the serialized form starts with "s-".
        assert!(json.contains("s-"));
    }
}
```

### 2. Rewrite `session/mod.rs` to be purely declarative

Replace entire content with:

```rust
//! Session identity types and persistence events.
//!
//! A [`SessionId`] uniquely identifies a chat session. It is generated
//! using UUID v4 and stored as an opaque string.
//!
//! [`SessionSaveRequested`] is emitted by the message queue handler to
//! trigger asynchronous session persistence via the actor system.
//! [`SessionLoadRequested`] is emitted when the user picks a session from
//! the session browser. [`SessionLoadCompleted`] carries the loaded data back.
//! [`SessionNew`] closes the session picker and starts a fresh session.

pub mod session_load_completed;
pub mod session_load_requested;
pub mod session_new;
pub mod session_save_requested;
mod session_id;

pub use session_id::SessionId;
pub use session_load_completed::SessionLoadCompleted;
pub use session_load_requested::SessionLoadRequested;
pub use session_new::SessionNew;
pub use session_save_requested::SessionSaveRequested;
```

### 3. Move `session/mod.rs` → `session.rs`

```
mv session/mod.rs session.rs
```

### 4. Update VCS (fossil rm old, fossil add new)

## Implementation Order

1. Create `session/session_id.rs`
2. Rewrite `session/mod.rs`
3. Move `session/mod.rs` → `session.rs`
4. Run `just check`
5. Fossil: `rm` old, `add` new

## Acceptance Criteria

- [x] `session/session_id.rs` exists and contains `SessionId` struct, impls, and tests
- [x] `session/mod.rs` no longer exists
- [x] `session.rs` exists and is purely declarative (no inline types)
- [x] `just check` passes

---

## Review: Phase 2 — Extract inline code from session/mod.rs and convert

### Changes

Extracted `SessionId` (struct, impls, and tests) from `session/mod.rs` into `session/session_id.rs`. Rewrote `session/mod.rs` to be purely declarative, then moved it to `session.rs`.

### Divergence Summary

None. Implemented exactly as planned.

### Verification

- Confirmed `session/session_id.rs` exists with `SessionId` and tests.
- Confirmed `session/mod.rs` no longer exists.
- Confirmed `session.rs` is purely declarative.
- `just check` passed.

### Risks

None. All re-exports preserved — `lib.rs` `pub use session::SessionId` still resolves.

### Next Steps

Phase 3: Extract inline code from `tab/mod.rs` and convert.
