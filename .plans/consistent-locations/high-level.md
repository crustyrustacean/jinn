# consistent-locations: Colocate struct definitions with their impl blocks, eliminate mod.rs

## Problem

After the module splitting, some files define struct A but also contain `impl B` for a completely different struct:

| File | Defines | Foreign impl |
|------|---------|-------------|
| `autocomplete.rs` | `AutocompleteState`, `AutocompleteMatch` | `impl ChatInputBoxState` (143 lines) |
| `builder.rs` | `TuiAppBuilder` | `impl TuiApp` (6 lines) |
| `pinned_pane.rs` | `PaneFocus`, `CHAT_PANE` | `impl TuiApp` (55 lines) |

Additionally, all `mod.rs` files should use the `module.rs` + `module/` pattern instead.

## User Decisions

- **Actor handler splits are fine** — domain-specific handler methods in separate files is acceptable. The problem is only when a file defines struct A but has `impl B`.
- **`chat_session/state/` split is fine** — domain-specific impl blocks for `ChatSessionState` in child modules is acceptable.
- **State structs get dedicated `state/` modules** — every struct with `State` in its name gets its own file inside a `state/` directory.
- **`AutocompleteMatch` goes into `lib.rs`** — it's not a State struct, so it stays in the crate root.
- **`PaneFocus` goes into `app.rs`** — collapse `pinned_pane.rs` entirely after moving foreign impl.
- **No `mod.rs`** — use `module.rs` + `module/` pattern everywhere.

## Phases

- [x] Phase 1: `nsslice-chat-input-box-protocol` — create `state/` module, fix foreign impl
  - [x] Create `state/chat_input_box.rs` — `ChatInputBoxState` struct + ALL its impls (from both `lib.rs` and `autocomplete.rs`)
  - [x] Create `state/autocomplete.rs` — `AutocompleteState` struct + its impl only
  - [x] Create `state.rs` — `mod` declarations + re-exports
  - [x] Move `AutocompleteMatch` into `lib.rs`
  - [x] Delete `autocomplete.rs`
  - [x] Commit

- [x] Phase 2: `nullslop-tui/app` — fix foreign impl, collapse `pinned_pane.rs`, eliminate `mod.rs`
  - [x] Rename `app/mod.rs` → `app.rs`
  - [x] Move `impl TuiApp` from `builder.rs` and `pinned_pane.rs` into `app.rs`
  - [x] Move `PaneFocus` + `CHAT_PANE` from `pinned_pane.rs` into `app.rs`
  - [x] Delete `pinned_pane.rs`
  - [x] `builder.rs` keeps only `TuiAppBuilder` + its impl
  - [x] `signals.rs` unchanged
  - [x] Commit

- [x] Phase 3: Eliminate all remaining `mod.rs` files
  - [x] `nsslice-session-management/src/actor/mod.rs` → `actor.rs`
  - [x] `nsslice-session-management/src/actor/handlers/mod.rs` → `handlers.rs`
  - [x] `nsslice-session-management/src/persistence/mod.rs` → `persistence.rs`
  - [x] `nsslice-context/src/actor/mod.rs` → `actor.rs`
  - [x] `nsslice-context/src/actor/handlers/mod.rs` → `handlers.rs`
  - [x] `nullslop-component/src/chat_session/mod.rs` → `chat_session.rs`
  - [x] `nullslop-component/src/chat_session/state/mod.rs` → `state.rs`
  - [x] `nsslice-session-management-protocol/src/session_store/mod.rs` → `session_store.rs`
  - [x] `nsslice-context-protocol/src/strategy/mod.rs` → `strategy.rs`
  - [x] `nullslop-component/src/prompt_template/mod.rs` → `prompt_template.rs`
  - [x] Commit

## Acceptance Criteria

- No file defines struct A and has `impl B` for a different struct
- Every struct with `State` in its name has a dedicated file in a `state/` module
- No `mod.rs` files exist — all use `module.rs` + `module/` pattern
- `just test` and `just lint` pass after each commit
