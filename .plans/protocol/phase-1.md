# Phase 1: Create `nullslop-domain` skeleton

## Problem

We need to create the new `nullslop-domain` crate that will absorb all 26 existing crates. This phase creates the crate skeleton with a merged `Cargo.toml` and empty module structure so subsequent phases can fill in the code.

## What Moves / What Stays

**Moves:** Nothing yet — this phase only creates the new crate.

**Stays:** All existing crates remain untouched until later phases.

## File Changes

### 1. CREATE `crates/common/nullslop-domain/Cargo.toml`

Merged deps from all 26 source crates. Union of all external and internal deps.

```toml
[package]
name = "nullslop-domain"
version = "0.1.0"
edition = "2024"

[features]
default = []
which-key = ["ratatui-which-key"]

[dependencies]
# Internal deps
nullslop-actor = { workspace = true }
nullslop-actor-host = { workspace = true }
nullslop-component = { workspace = true }
nullslop-services = { workspace = true }
nullslop-providers = { workspace = true }
nullslop-selection-widget = { workspace = true }
nullslop-prompt-template = { workspace = true }
nullslop-protocol-derive = { workspace = true }

# External deps
serde = { workspace = true }
serde_json = { workspace = true }
jiff = { workspace = true }
derive_more = { workspace = true }
ratatui-which-key = { workspace = true, optional = true }
uuid = { workspace = true }
ratatui = { workspace = true }
async-trait = { workspace = true }
wherror = { workspace = true }
error-stack = { workspace = true }
dirs = { workspace = true }
tokio = { workspace = true }
tracing = { workspace = true }
unicode-segmentation = { workspace = true }
throbber-widgets-tui = { workspace = true }
humantime = { workspace = true }
fuzzy-matcher = { workspace = true }
parking_lot = { workspace = true }
kanal = { workspace = true }

[dev-dependencies]
rstest = { workspace = true }
tokio = { workspace = true, features = ["test-util"] }
tempfile = { workspace = true }

[lints]
workspace = true
```

### 2. CREATE `crates/common/nullslop-domain/src/lib.rs`

Module declarations + re-exports. Modules are empty stubs for now; they get filled in later phases.

```rust
//! The domain layer — protocol types, actors, intents, and UI elements.
//!
//! This crate consolidates all domain types and logic into a single crate:
//!
//! - **Protocol types** (`protocol/`) — Command/Event mega-enums, foundational
//!   value types (ChatEntry, SessionId, Key, Mode, etc.)
//! - **Component UI** (`component_ui/`) — UiElement trait and registry
//! - **Domain slices** — actors, intents, UI elements, and state for each
//!   domain (provider, session, context, tools, etc.)
//!
//! Protocol types are re-exported at the crate root for convenience.

pub mod component_ui;
pub mod protocol;

// Domain slices — filled in by later phases
pub mod char_counter;
pub mod chat_entry_selection;
pub mod chat_input_box;
pub mod chat_log;
pub mod chat_session;
pub mod context;
pub mod dashboard;
pub mod echo;
pub mod global;
pub mod llm;
pub mod navigation;
pub mod picker;
pub mod pinned_panel;
pub mod provider;
pub mod session;
pub mod shutdown;
pub mod status_bar;
pub mod tools;
```

### 3. CREATE stub `mod.rs` files for each module

Each module directory gets a `mod.rs` with just a module-level doc comment so the crate compiles.

### 4. MODIFY workspace `Cargo.toml`

- Add `nullslop-domain` to `[workspace.dependencies]`
- Do NOT remove old crates yet (they're still in use)

## Implementation Order

1. Create `crates/common/nullslop-domain/Cargo.toml`
2. Create `crates/common/nullslop-domain/src/lib.rs`
3. Create all stub `mod.rs` files
4. Add `nullslop-domain` to workspace `Cargo.toml`
5. Run `just check` to verify compilation

## Acceptance Criteria

- [x] `crates/common/nullslop-domain/Cargo.toml` exists with all required deps
- [x] `crates/common/nullslop-domain/src/lib.rs` exists with module declarations
- [x] All stub module directories and `mod.rs` files exist (20 modules)
- [x] `nullslop-domain` added to workspace `[workspace.dependencies]`
- [x] `cargo check -p nullslop-domain` passes
- [x] No existing crates are modified or removed

---

## Review: Phase 1 — Create `nullslop-domain` skeleton

### Changes

Created the `nullslop-domain` crate at `crates/common/nullslop-domain/` with:
- `Cargo.toml` with merged deps from all 26 source crates
- `src/lib.rs` with 20 module declarations
- 20 stub `mod.rs` files, each with a doc comment
- Workspace entry in root `Cargo.toml`

### Divergence Summary

None. Implemented exactly as planned.

### Verification

- `cargo check -p nullslop-domain` passes
- `just check` (full workspace) passes
- All 20 stub files confirmed present

### Risks

Some dependencies in `Cargo.toml` may not actually be needed after the full merge (e.g., deps that were only used by a subset of slices). Can be pruned in a later pass.

### Next Steps

Phase 2: Move `nullslop-protocol` contents into `nullslop-domain/src/protocol/`.
