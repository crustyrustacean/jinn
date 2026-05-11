# VSA-3: Extract Navigation, Session & Model, and Global Slices

## Problem

After VSA-2, `nullslop-intent/handler.rs` still has 16 intents with inline logic or helper functions: 8 navigation, 3 session/model, and 5 mode/app. This plan extracts the first three groups into dedicated slice crates, leaving only the mode/app intents for VSA-4.

VSA-3 is the last "bulk extraction" — VSA-4 handles the remaining protocol-level changes (SetMode split, Interrupt session_id, NormalEscape move).

## Decisions

### Three new slice crates

- **`nsslice-navigation`** — scroll, tab, edit-input. Pure state mutation, no validators, no commands. Simplest extraction.
- **`nsslice-session-management`** — session new, refresh models, rescan prompt templates. Has 3 validators (1 fallible, 2 semi-fallible). Deletes `validators/chat_entry.rs` entirely.
- **`nsslice-global`** — quit, toggle whichkey, interrupt. The "cross-cutting app actions" slice. Requires moving `validate_interrupt` + `InterruptError` out of `nsslice-chat-input-box` (slices cannot depend on each other). Also moves `cancel_stream_and_drain` helper, which is shared with `SetMode` (stays in `nullslop-intent` until VSA-4).

### `validate_interrupt` relocation

`validate_interrupt` is currently defined in `nsslice-chat-input-box/src/validator.rs` because it checks chat input state (empty buffer + idle session). But the Interrupt intent is a global action, and the global slice can't depend on the chat-input-box slice. Solution: move `validate_interrupt`, `InterruptError`, and its 3 tests from `nsslice-chat-input-box` to `nsslice-global`.

After the move, `nsslice-chat-input-box/src/validator.rs` retains only `SubmitMessageError` + `validate_submit_message` and `AutocompleteConfirmError` + `validate_autocomplete_confirm` plus their 5 tests. The module doc comment updates from "message submission, autocomplete confirmation, and interrupt" to "message submission and autocomplete confirmation."

### `cancel_stream_and_drain` shared helper

This helper is used by both `handle_interrupt` (moving to `nsslice-global`) and `handle_set_mode` (staying in `nullslop-intent` until VSA-4). It moves to `nsslice-global` as a public function so `nullslop-intent` can call `nsslice_global::intent::cancel_stream_and_drain(state)` for the `SetMode` handler.

### What stays in `nullslop-intent` after VSA-3

Only mode/app handlers, waiting for VSA-4:
- `handle_set_mode` — uses `cancel_stream_and_drain` from `nsslice-global`
- `handle_normal_escape` — moves to `nsslice-chat-input-box` in VSA-4
- `app.rs` validators (3 infallible) — deleted in VSA-4
- Picker re-dispatch logic for keymap confirm
- `tui_signals.clear()` preamble

---

## Acceptance Criteria

1. Three new slice crates exist: `nsslice-navigation`, `nsslice-session-management`, `nsslice-global`
2. `nsslice-navigation` has 8 handler functions + 8 tests, all passing independently
3. `nsslice-session-management` has 3 handler functions + 3 validators + 9 tests (4 validator + 5 handler), all passing independently
4. `nsslice-global` has 3 handler functions + 1 validator + 2 infallible validators + `cancel_stream_and_drain` helper + appropriate tests
5. `nsslice-chat-input-box` no longer has `validate_interrupt`, `InterruptError`, or interrupt tests
6. `nullslop-intent/src/validators/chat_entry.rs` deleted, `validators/mod.rs` updated
7. `nullslop-intent/handler.rs` delegates all extracted intents to slice crates
8. `handle_set_mode` in `nullslop-intent` calls `nsslice_global::intent::cancel_stream_and_drain`
9. `cargo test --workspace` passes — no regressions

---

## Implementation Phases

- [x] Phase 1: Navigation → `nsslice-navigation` ✅
  - [x] Create `crates/slices/nsslice-navigation/Cargo.toml` — depends on `nullslop-component`, `nullslop-protocol`; dev-dep on `rstest`
  - [x] Create `nsslice-navigation/src/lib.rs` — `pub mod intent;` (no validators, no element, no register())
  - [x] Create `nsslice-navigation/src/intent.rs` — 8 public handler functions + constants + 8 tests:
    - `SCROLL_STEP: u16 = 10`, `MOUSE_SCROLL_STEP: u16 = 3` (moved from IntentHandler)
    - `handle_scroll_up(state) -> IntentResult`
    - `handle_scroll_down(state) -> IntentResult`
    - `handle_mouse_scroll_up(state) -> IntentResult`
    - `handle_mouse_scroll_down(state) -> IntentResult`
    - `handle_scroll_to_top(state) -> IntentResult`
    - `handle_scroll_to_bottom(state) -> IntentResult`
    - `handle_switch_tab(state, direction: TabDirection) -> IntentResult`
    - `handle_edit_input(state) -> IntentResult`
    - 8 tests moved from `handler_tests.rs`
  - [x] Add `nsslice-navigation` to root `Cargo.toml` (workspace members, workspace.dependencies, dependencies)
  - [x] Add `nsslice-navigation` dep to `nullslop-intent/Cargo.toml`
  - [x] Update `nullslop-intent/src/handler.rs` — replace 8 inline Navigation match arms with delegations to `nsslice_navigation::intent::*`; remove `SCROLL_STEP` and `MOUSE_SCROLL_STEP` constants
  - [x] Remove 8 navigation tests from `nullslop-intent/src/handler_tests.rs`
  - [x] Run `cargo test --workspace`

- [x] Phase 2: Session & Model → `nsslice-session-management` ✅
  - [x] Create `crates/slices/nsslice-session-management/Cargo.toml` — depends on `nullslop-component`, `nullslop-protocol`, `wherror`; dev-deps on `rstest`
  - [x] Create `nsslice-session-management/src/lib.rs` — `pub mod intent; pub mod validator;` (no element, no register())
  - [x] Create `nsslice-session-management/src/validator.rs` — move entire contents of `nullslop-intent/src/validators/chat_entry.rs`:
    - `RefreshModelsError` + `validate_refresh_models`
    - `RescanPromptTemplatesError` + `validate_rescan_prompt_templates`
    - `SessionNewError` + `validate_session_new`
    - 4 validator tests
  - [x] Create `nsslice-session-management/src/intent.rs` — 3 public handler functions + 5 tests:
    - `handle_session_new(state) -> IntentResult`
    - `handle_refresh_models(state) -> IntentResult`
    - `handle_rescan_prompt_templates(state) -> IntentResult`
    - 5 handler tests moved from `handler_tests.rs`
  - [x] Add `nsslice-session-management` to root `Cargo.toml` (workspace members, workspace.dependencies, dependencies)
  - [x] Add `nsslice-session-management` dep to `nullslop-intent/Cargo.toml`
  - [x] Update `nullslop-intent/src/handler.rs` — replace 3 session/model match arms with delegations; remove `handle_session_new`, `handle_refresh_models`, `handle_rescan_prompt_templates` helper functions
  - [x] Remove 5 session/model tests from `nullslop-intent/src/handler_tests.rs`; clean unused imports (`ProviderState`)
  - [x] Delete `nullslop-intent/src/validators/chat_entry.rs`
  - [x] Update `nullslop-intent/src/validators/mod.rs` — remove `pub mod chat_entry;`
  - [x] Update `nullslop-intent/src/handler.rs` — remove `use crate::validators::chat_entry` import
  - [x] Run `cargo test --workspace`

- [x] Phase 3: Global → `nsslice-global`
  - [x] Create `crates/slices/nsslice-global/Cargo.toml` — depends on `nullslop-component`, `nullslop-protocol`, `wherror`; dev-deps on `rstest`
  - [x] Create `nsslice-global/src/lib.rs` — `pub mod intent; pub mod validator;` (no element, no register())
  - [x] Create `nsslice-global/src/validator.rs` — move validators from two sources:
    - From `nullslop-intent/src/validators/app.rs` (entire file):
      - `validate_quit` (infallible)
      - `validate_toggle_whichkey` (infallible)
      - `validate_normal_escape` (infallible) — note: this validator stays here even though NormalEscape handler itself stays in nullslop-intent until VSA-4. The validator is trivial and colocating it with the other app validators is fine.
    - From `nsslice-chat-input-box/src/validator.rs` (partial):
      - `InterruptError` enum
      - `validate_interrupt` function
      - 3 interrupt tests (`interrupt_succeeds_with_non_empty_buffer`, `interrupt_succeeds_with_active_stream`, `interrupt_fails_with_empty_buffer_and_idle_session`)
  - [x] Update `nsslice-chat-input-box/src/validator.rs` — remove `InterruptError`, `validate_interrupt`, and 3 interrupt tests. Update module doc comment from "message submission, autocomplete confirmation, and interrupt" to "message submission and autocomplete confirmation."
  - [x] Create `nsslice-global/src/intent.rs` — 3 public handler functions + `cancel_stream_and_drain` + tests:
    - `handle_quit(state) -> IntentResult` — validate + set `should_quit`
    - `handle_toggle_whichkey(state) -> IntentResult` — validate + set signal
    - `handle_interrupt(state) -> IntentResult` — validate + deactivate autocomplete + cancel stream or reset input
    - `pub fn cancel_stream_and_drain(state: &mut AppState)` — moved from `nullslop-intent`, shared with `SetMode`
    - Handler tests for quit, toggle whichkey, and interrupt moved from `handler_tests.rs`
  - [x] Add `nsslice-global` to root `Cargo.toml` (workspace members, workspace.dependencies, dependencies)
  - [x] Add `nsslice-global` dep to `nullslop-intent/Cargo.toml`
  - [x] Update `nullslop-intent/src/handler.rs`:
    - Replace Quit, ToggleWhichkey, Interrupt match arms with delegations to `nsslice_global::intent::*`
    - Remove `handle_interrupt` helper function
    - Update `handle_set_mode` to call `nsslice_global::intent::cancel_stream_and_drain(state)` instead of local function
    - Remove local `cancel_stream_and_drain` function
    - Update Quit/ToggleWhichkey to use `nsslice_global::validator::*` instead of `app::*`
    - Keep NormalEscape using `app::validate_normal_escape` until VSA-4 (validator lives in `nsslice-global` now, but handler stays temporarily)
  - [x] Remove moved tests from `nullslop-intent/src/handler_tests.rs` (quit, toggle whichkey, all interrupt tests — but keep set_mode, normal_escape, and signal tests)
  - [x] Clean imports in `handler.rs` — remove `CancelStream` import if only used by moved code; keep if `handle_set_mode` still uses it (it does — `handle_set_mode` constructs `CancelStream`)
  - [x] Run `cargo test --workspace`

---

## Post-VSA-3 State of `nullslop-intent`

| Category | Intents | Status |
|----------|---------|--------|
| Chat Input (13) | All delegated to `nsslice-chat-input-box` | Done since VSA-1 |
| Navigation (8) | All delegated to `nsslice-navigation` | **VSA-3 Phase 1** |
| Session/Model (3) | All delegated to `nsslice-session-management` | **VSA-3 Phase 2** |
| Global (3) | Quit, ToggleWhichkey, Interrupt → `nsslice-global` | **VSA-3 Phase 3** |
| Mode & App (2) | SetMode, NormalEscape — stay for VSA-4 | Deferred |
| Picker confirm re-dispatch | Stays (needs `IntentHandler::handle`) | Permanent |
| `tui_signals.clear()` | Stays (dispatch hub responsibility) | Permanent |
| `validators/app.rs` | Still has `validate_normal_escape` (Quit/Whichkey now in `nsslice-global`) | VSA-4 deletes |

### `crates/slices/` — All feature slices after VSA-3 (12 total)

| Slice | Element | Intent | Validator |
|-------|---------|--------|-----------|
| `nsslice-char-counter` | ✅ | — | — |
| `nsslice-status-bar` | ✅ | — | — |
| `nsslice-dashboard` | ✅ | ✅ (4) | — |
| `nsslice-chat-log` | ✅ | — | — |
| `nsslice-provider` | ✅ | — | — |
| `nsslice-pinned-panel` | ✅ | ✅ (11) | ✅ (5) |
| `nsslice-chat-input-box` | ✅ | ✅ (13) | ✅ (2) |
| `nsslice-picker` | — | ✅ (9) | ✅ (9) |
| `nsslice-chat-entry-selection` | — | ✅ (3) | ✅ (3) |
| `nsslice-navigation` | — | ✅ (8) | — |
| `nsslice-session-management` | — | ✅ (3) | ✅ (3) |
| `nsslice-global` | — | ✅ (3) | ✅ (4) |
