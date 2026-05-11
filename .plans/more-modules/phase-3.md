# Phase 3: Extract inline code from `tab/mod.rs` and convert

## Problem

`tab/mod.rs` contains inline code: the `TabDirection` enum and its `Display` impl. To convert it to `tab.rs`, we must first extract the inline code into a dedicated file, making the module root purely declarative.

## What Moves

- `TabDirection` enum and `impl Display for TabDirection` move from `tab/mod.rs` → `tab/tab_direction.rs`
- `tab/mod.rs` is then rewritten to be purely declarative and moved to `tab.rs`

## File Changes

### 1. Create `tab/tab_direction.rs` (new file)

```rust
//! Direction for tab cycling.

use serde::{Deserialize, Serialize};

/// Direction for tab cycling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TabDirection {
    /// Move to the next tab (wrapping).
    Next,
    /// Move to the previous tab (wrapping).
    Prev,
}

impl std::fmt::Display for TabDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Next => write!(f, "next"),
            Self::Prev => write!(f, "prev"),
        }
    }
}
```

### 2. Rewrite `tab/mod.rs` to be purely declarative

```rust
//! Tab domain: tab management types, active tab state, and tab navigation commands.

mod active_tab;
mod command;
mod tab_direction;

pub use active_tab::ActiveTab;
pub use tab_direction::TabDirection;
```

### 3. Move `tab/mod.rs` → `tab.rs`

### 4. Update VCS

## Implementation Order

1. Create `tab/tab_direction.rs`
2. Rewrite `tab/mod.rs`
3. Move `tab/mod.rs` → `tab.rs`
4. Run `just check`
5. Fossil: `rm` old, `add` new

## Acceptance Criteria

- [x] `tab/tab_direction.rs` exists and contains `TabDirection` enum and `Display` impl
- [x] `tab/mod.rs` no longer exists
- [x] `tab.rs` exists and is purely declarative (no inline types)
- [x] `just check` passes

---

## Review: Phase 3 — Extract inline code from tab/mod.rs and convert

### Changes

Extracted `TabDirection` enum and `Display` impl from `tab/mod.rs` into `tab/tab_direction.rs`. Rewrote `tab/mod.rs` to be purely declarative, then moved it to `tab.rs`.

### Divergence Summary

None. Implemented exactly as planned.

### Verification

- Confirmed `tab/tab_direction.rs` exists with `TabDirection` and `Display` impl.
- Confirmed `tab/mod.rs` no longer exists.
- Confirmed `tab.rs` is purely declarative.
- `just check` passed.

### Risks

None. All re-exports preserved.

### Next Steps

Phase 4: Final validation — confirm no mod.rs files remain, run full test suite and lint.
