# chat-session: Extract ChatSessionState into nsslice-chat-session-protocol

## Problem

`ChatSessionState` (plus `SessionCore`, `SessionUi`, the test builder, and ~500 lines of tests) lives inside `nullslop-component` at `crates/common/nullslop-component/src/chat_session/`. The long-term goal is to reduce `nullslop-component` to just `AppState`-related types so it can eventually be renamed. Extracting `ChatSessionState` into its own protocol crate is the next step toward that goal.

This is a protocol-only crate extraction — `ChatSessionState` is state-only (no UI element, no intent handler). It follows the same pattern as `nsslice-chat-input-box-protocol`.

## Naming Decision

`nsslice-chat-session-protocol` — follows the established pattern (`nsslice-chat-input-box-protocol`, `nsslice-session-management-protocol`, etc.).

## Dependencies

The new crate depends on:
- `nullslop-protocol` — for `ChatEntry`, `ChatEntryId`, `ChatEntryKind`, `PinPosition`, `PromptStrategyId`
- `nsslice-chat-input-box-protocol` — for `ChatInputBoxState` used in `SessionUi`
- `serde_json` — for `JsonValue` used in `SessionCore.strategy_state`

Dev-dependencies:
- `rstest` — for test macros
- `nullslop-protocol` (already a regular dependency, so no extra dev-dep needed)

## Key Design Decisions

1. **Backward-compatible re-export**: `nullslop-component/src/lib.rs` will re-export `ChatSessionState` from the new crate so downstream consumers don't break. Individual crates can be migrated to import directly from the new crate over time.

2. **Builder stays `#[cfg(test)]`**: The `ChatSessionStateBuilder` is only used by co-located tests in the same crate. It stays `#[cfg(test)]` with no public export.

3. **Test field access**: Some tests in `state_tests.rs` directly access `session.core.*` and `session.ui.*` (private fields). This works because the test module is declared inside `state.rs` via `#[path = "state_tests.rs"]`. The same pattern will work in the new crate.

## Consumers to Update

| Consumer | Current Import | New Import |
|---|---|---|
| `nullslop-component/src/app_state.rs` | `use crate::chat_session::ChatSessionState` | `use nsslice_chat_session_protocol::ChatSessionState` |
| `nullslop-component/src/lib.rs` | `pub mod chat_session; pub use chat_session::ChatSessionState` | `pub use nsslice_chat_session_protocol::ChatSessionState` |
| `nsslice-session-management/src/intent.rs` | `nullslop_component::chat_session::ChatSessionState::new()` | `nsslice_chat_session_protocol::ChatSessionState::new()` (add dep + import) |
| `src/session_conversion.rs` | `use nullslop_component::chat_session::ChatSessionState` | `use nsslice_chat_session_protocol::ChatSessionState` (add dep + import) |

## Phases

- [x] Phase 1: Create `nsslice-chat-session-protocol` crate
  - [x] Create `crates/slices/nsslice-chat-session-protocol/Cargo.toml` with dependencies: `nullslop-protocol`, `nsslice-chat-input-box-protocol`, `serde_json`; dev-dep: `rstest`
  - [x] Create `crates/slices/nsslice-chat-session-protocol/src/lib.rs` — move `ChatSessionState`, `SessionCore`, `SessionUi` from `nullslop-component/src/chat_session/state.rs`
  - [x] Create `crates/slices/nsslice-chat-session-protocol/src/tests.rs` — move all tests from `nullslop-component/src/chat_session/state_tests.rs`, updating imports
  - [x] Move the `#[cfg(test)]` builder (`ChatSessionStateBuilder`, `BuilderOp`) into the new crate's `lib.rs`

- [x] Phase 2: Wire up the new crate and update consumers
  - [x] Add `nsslice-chat-session-protocol` to workspace `Cargo.toml` `[workspace.dependencies]` and `[workspace.members]`
  - [x] Add `nsslice-chat-session-protocol` as a dependency of `nullslop-component` in its `Cargo.toml`
  - [x] Update `nullslop-component/src/lib.rs`: remove `pub mod chat_session`, add `pub use nsslice_chat_session_protocol::ChatSessionState`
  - [x] Update `nullslop-component/src/app_state.rs`: change `use crate::chat_session::ChatSessionState` to `use nsslice_chat_session_protocol::ChatSessionState`
  - [x] Add `nsslice-chat-session-protocol` dep to `nsslice-session-management/Cargo.toml`; update `nsslice-session-management/src/intent.rs` to import `ChatSessionState` directly
  - [x] Add `nsslice-chat-session-protocol` dep to root `Cargo.toml`; update `src/session_conversion.rs` to import from new crate
  - [x] Run `just check`

- [x] Phase 3: Remove old module and verify
  - [x] Delete `crates/common/nullslop-component/src/chat_session/` directory
  - [x] Run `just check` then `just test` — all must pass

## Acceptance Criteria

- `ChatSessionState`, `SessionCore`, `SessionUi` live in `nsslice-chat-session-protocol`
- `nullslop-component` re-exports `ChatSessionState` for backward compatibility
- All existing tests pass (moved tests + all downstream crate tests)
- `nullslop-component/src/chat_session/` directory no longer exists
- The new crate follows the same conventions as existing protocol slices
