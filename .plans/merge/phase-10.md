# Phase 10: Migrate `ChatInputBoxState`

## Problem

`ChatInputBoxState` (588 lines + 451 lines tests) lives in `nullslop-component/src/chat_input_box/`. Moving it to `nsslice-chat-input-box-protocol` shrinks `nullslop-component` significantly. This is the largest state migration.

## What Moves

- `ChatInputBoxState`, `AutocompleteMatch`, `AutocompleteState`, all tests → `nsslice-chat-input-box-protocol/src/lib.rs`

## What Stays

- `nsslice-chat-input-box/src/` — stays, updates imports
- `AppState` field `chat_input` accessors — stay, typed via protocol crate

## File Changes

### 1. NEW `crates/slices/nsslice-chat-input-box-protocol/Cargo.toml`
```toml
[package]
name = "nsslice-chat-input-box-protocol"
version = "0.1.0"
edition = "2024"

[dependencies]
unicode-segmentation = { workspace = true }

[dev-dependencies]
rstest = { workspace = true }

[lints]
workspace = true
```

### 2. NEW `crates/slices/nsslice-chat-input-box-protocol/src/lib.rs`
Copy from `nullslop-component/src/chat_input_box/state.rs` + `state_tests.rs`.

### 3. MODIFY `crates/nullslop-component/Cargo.toml`
Add: `nsslice-chat-input-box-protocol = { workspace = true }`
Remove: `unicode-segmentation = { workspace = true }` if no longer needed

### 4. MODIFY `crates/nullslop-component/src/lib.rs`
Remove `pub mod chat_input_box;`, change re-export.

### 5. MODIFY `crates/nullslop-component/src/app_state.rs`
Change import.

### 6. MODIFY `crates/slices/nsslice-chat-input-box/Cargo.toml`
Add: `nsslice-chat-input-box-protocol = { workspace = true }`

### 7. MODIFY `crates/slices/nsslice-chat-input-box/src/` — update imports

### 8. MODIFY root `Cargo.toml`

### 9. DELETE `crates/nullslop-component/src/chat_input_box/` directory

- [x] `crates/slices/nsslice-chat-input-box-protocol/` exists with state + tests
- [x] `crates/nullslop-component/src/chat_input_box/` is deleted
- [x] `nullslop-component` re-exports `ChatInputBoxState` from protocol crate
- [x] `just check` passes
- [x] `just test` passes — all chat input box state tests pass from protocol crate

---

## Review: Phase 10 — Migrate `ChatInputBoxState`

### Changes

- Created `nsslice-chat-input-box-protocol` with `ChatInputBoxState`, `AutocompleteMatch`, `AutocompleteState`, and all tests (1,039 lines total)
- Updated `nullslop-component` to import and re-export from protocol crate
- Updated `nsslice-chat-input-box` intent and tests to import from protocol crate
- Updated `nullslop-component/src/chat_session/state.rs` to import `ChatInputBoxState` from protocol crate
- Deleted `nullslop-component/src/chat_input_box/` directory

### Divergence Summary

- Had to also update `nullslop-component/src/chat_session/state.rs` — `ChatSessionState` has a `ChatInputBoxState` field
- Tests were merged inline into lib.rs with `#[cfg(test)] mod tests` wrapper (original had separate `state_tests.rs` module)

### Verification

- `just check` — zero errors
- `just test` — all pass

### Risks

- None. This unblocks the deferred state migration for `ChatSessionState` → `nsslice-session-management-protocol` (from Phase 6).

### Next Steps

Proceed to Phase 11: Refactor spawning.
