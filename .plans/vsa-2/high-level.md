# VSA-2: Extract Dashboard Intents, Picker System, and Chat Entry Selection

## Problem

After VSA phases 1–6, seven feature slices have been extracted (`nsslice-char-counter`, `nsslice-status-bar`, `nsslice-dashboard`, `nsslice-chat-log`, `nsslice-provider`, `nsslice-pinned-panel`, `nsslice-chat-input-box`). However, `nsslice-dashboard` only has its element — the 4 dashboard intents still live in `nullslop-intent/handler.rs`. The picker system (~12 intents, 9 validators) and chat entry selection (3 intents, 1 fallible validator) are also still in `nullslop-intent`, making it the largest remaining un-extracted code.

This plan extracts 3 more feature groups into slice crates, reducing `nullslop-intent/handler.rs` from ~280 lines of match arms to ~80 lines.

## Decisions Made During Planning

### Single picker slice, not per-kind

The picker system handles 4 picker kinds (Provider, ContextAssembly, Keymap, Session) but all share the same intent variants and validator patterns. A single `nsslice-picker` crate is the right granularity. Sub-slices for individual picker kinds can be added later if needed.

### `RefreshModels`, `RescanPromptTemplates`, `SessionNew` stay in `nullslop-intent`

These are picker-scope intents but are arguably app-level (create sessions, refresh models, rescan templates). Per user decision, they stay in `nullslop-intent` for now and will move later.

### `handle_interrupt`, `handle_set_mode`, `handle_normal_escape` stay in `nullslop-intent`

These are cross-cutting handlers (cancel streams, drain queues, touch picker state, clear selections). They stay permanently in `nullslop-intent` per plan decision.

### `confirm_keymap` avoids circular dependency via return value

`confirm_keymap` currently calls `IntentHandler::handle()` recursively (re-dispatches the selected keymap intent). Moving this into the slice would create a circular dep (`nullslop-intent` ↔ `nsslice-picker`). **Solution**: `handle_picker_confirm` returns `(IntentResult, Option<Intent>)`. For keymap confirm, it returns `(IntentResult::empty(), Some(intent))`. The caller in `nullslop-intent` handles the re-dispatch. This avoids circular deps and keeps the slice testable.

### Navigation intents stay for now but are NOT cross-cutting

The scroll intents (`ScrollUp/Down/ToTop/ToBottom`, `MouseScrollUp/Down`) always scroll the active chat session — they are chat-log-specific, not cross-cutting. Each scrollable area has its own dedicated intents (dashboard has `DashboardSelectDown/Up/First/Last`, pinned panel has `PinnedPanelSelectDown/Up`). These will be extracted to a `nsslice-navigation` slice in a future plan.

### `SwitchTab` and `EditInput` also stay for now

These touch `active_tab` and `tui_signals` directly. They'll move to a future navigation slice.

### Established patterns to follow

From VSA phase 1+2 execution notes: **every new slice must also update `nullslop-tui`** — add the new slice dependency to `crates/nullslop-tui/Cargo.toml` and add `register()` calls at all 3 `nullslop_component::register_all()` call sites in `crates/nullslop-tui/src/app.rs`. However, `nsslice-picker` and `nsslice-chat-entry-selection` have **no elements** (no `register()` function), so they only need to be added as dependencies where used.

From VSA phase 5+6: workspace entries go in root `Cargo.toml`. Registration happens at 5 sites total (2 in `src/app.rs`, 3 in `crates/nullslop-tui/src/app.rs`).

### Dashboard validator is empty — just delete it

`nullslop-intent/src/validators/dashboard.rs` contains only a module-level doc comment. No functions, no tests. It can simply be deleted and removed from `validators/mod.rs`.

---

## Acceptance Criteria

1. `nsslice-dashboard` gains `intent.rs` with 4 handler functions + 4 tests
2. New `nsslice-picker` crate with `intent.rs` + `validator.rs`, all tests passing independently
3. New `nsslice-chat-entry-selection` crate with `intent.rs` + `validator.rs`, all tests passing independently
4. `nullslop-intent/handler.rs` delegates to slice functions for all extracted intents
5. `nullslop-intent/src/validators/dashboard.rs` deleted, `picker.rs` deleted, `chat_entry.rs` reduced to only `RefreshModels`/`RescanPromptTemplates`/`SessionNew` validators
6. `cargo test --workspace` passes
7. Each slice crate is independently testable (`cargo test -p nsslice-dashboard`, etc.)

---

## Implementation Phases

- [x] Phase 1: Dashboard intents → `nsslice-dashboard`
  - [x] Add `nullslop-protocol` dependency to `nsslice-dashboard/Cargo.toml` (for `IntentResult`)
  - [x] Create `nsslice-dashboard/src/intent.rs` with 4 handler functions + 4 tests moved from `nullslop-intent/src/handler_tests.rs`:
    - `handle_select_down(state) -> IntentResult`
    - `handle_select_up(state) -> IntentResult`
    - `handle_select_first(state) -> IntentResult`
    - `handle_select_last(state) -> IntentResult`
  - [x] Update `nsslice-dashboard/src/lib.rs` — add `pub mod intent;`
  - [x] Update `nullslop-intent/src/handler.rs` — 4 dashboard match arms delegate to `nsslice_dashboard::intent::*`
  - [x] Delete `nullslop-intent/src/validators/dashboard.rs` (empty file — just a doc comment)
  - [x] Update `nullslop-intent/src/validators/mod.rs` — remove `pub mod dashboard;`
  - [x] Move 4 dashboard tests from `nullslop-intent/src/handler_tests.rs` to `nsslice-dashboard/src/intent.rs`
  - [x] Run `cargo test --workspace`

- [x] Phase 2: Picker system → new `nsslice-picker`
  - [x] Create `crates/slices/nsslice-picker/Cargo.toml` — depends on `nullslop-component`, `nullslop-protocol`, `wherror`; dev-deps on `rstest`, `jiff`
  - [x] Create `nsslice-picker/src/lib.rs` — `pub mod intent; pub mod validator;` (no element, no `register()`)
  - [x] Create `nsslice-picker/src/validator.rs` — moved entire contents of `nullslop-intent/src/validators/picker.rs`:
    - 7 infallible validators
    - 2 fallible validators + 2 error enums
    - 8 validator tests
  - [x] Create `nsslice-picker/src/intent.rs` — moved picker handler functions + tests:
    - 9 public handler functions (handle_open_picker, handle_insert_char, handle_backspace, handle_picker_confirm, handle_move_up, handle_move_down, handle_move_cursor_left, handle_move_cursor_right, handle_toggle_keymap_scope_filter)
    - 4 private confirm functions (confirm_provider, confirm_strategy, confirm_keymap, confirm_session)
    - PICKER_MAX_VISIBLE constant moved from IntentHandler
    - `handle_picker_confirm` returns `(IntentResult, Option<Intent>)` to avoid circular dep
    - 14 handler tests moved + 1 new test (`picker_confirm_keymap_returns_intent_for_redispatch`)
  - [x] Delete `nullslop-intent/src/validators/picker.rs`
  - [x] Update `nullslop-intent/src/validators/mod.rs` — removed `pub mod picker;`
  - [x] Update `nullslop-intent/src/handler.rs`:
    - 9 picker match arms delegate to `nsslice_picker::intent::*`
    - `PickerConfirm` handles `(IntentResult, Option<Intent>)` return for keymap re-dispatch
    - Removed `PICKER_MAX_VISIBLE` constant
    - Removed 7 helper functions: `handle_open_picker`, `handle_picker_confirm`, `confirm_provider`, `confirm_strategy`, `confirm_keymap`, `confirm_session`, `handle_toggle_keymap_scope_filter`
  - [x] Add `nsslice-picker` to workspace members, workspace.dependencies, and [dependencies] in root `Cargo.toml`
  - [x] Add `nsslice-picker` dependency to `nullslop-intent/Cargo.toml`
  - [x] Run `cargo test --workspace` — all 743+ tests pass

- [x] Phase 3: Chat Entry Selection → new `nsslice-chat-entry-selection`
  - [x] Create `crates/slices/nsslice-chat-entry-selection/Cargo.toml` — depends on `nullslop-component`, `nullslop-protocol`, `wherror`; dev-dep on `rstest`
  - [x] Create `nsslice-chat-entry-selection/src/lib.rs` — `pub mod intent; pub mod validator;` (no element, no `register()`)
  - [x] Create `nsslice-chat-entry-selection/src/validator.rs` — move from `nullslop-intent/src/validators/chat_entry.rs`:
    - `validate_chat_entry_select_next` (infallible)
    - `validate_chat_entry_select_prev` (infallible)
    - `validate_chat_entry_pin_selected` (fallible) + `ChatEntryPinSelectedError` enum
    - 3 validator tests: `pin_selected_succeeds_with_selected_entry`, `pin_selected_fails_with_empty_history`, `pin_selected_fails_with_no_selection`
  - [x] Create `nsslice-chat-entry-selection/src/intent.rs` — move from `nullslop-intent/src/handler.rs`:
    - 3 handler functions: `handle_select_next`, `handle_select_prev`, `handle_pin_selected`
    - 4 handler tests moved from `handler_tests.rs`
  - [x] Update `nullslop-intent/src/validators/chat_entry.rs` — **remove** the moved validators/tests, **keep**:
    - `validate_refresh_models` + `RefreshModelsError`
    - `validate_rescan_prompt_templates` + `RescanPromptTemplatesError`
    - `validate_session_new` + `SessionNewError`
    - Their 4 validator tests
  - [x] Update `nullslop-intent/src/handler.rs` — 3 chat-entry match arms delegate to `nsslice_chat_entry_selection::intent::*`
  - [x] Remove `handle_chat_entry_pin_selected` helper function from `nullslop-intent/src/handler.rs`
  - [x] Add `nsslice-chat-entry-selection` to workspace members in root `Cargo.toml`
  - [x] Add `nsslice-chat-entry-selection` dependency to `nullslop-intent/Cargo.toml`
  - [x] Run `cargo test --workspace`

---

## Post-Phase-3 State of the Codebase

### `nullslop-intent` — What remains

| Category | Intents | Lines (approx) |
|----------|---------|---------|
| Navigation (scroll, tab, edit) | 8 | ~20 |
| Mode & App (quit, interrupt, set-mode, whichkey, escape) | 5 | ~50 |
| `RefreshModels`, `RescanPromptTemplates`, `SessionNew` | 3 | ~30 |
| `confirm_keymap` re-dispatch | — | ~5 |
| `cancel_stream_and_drain` helper | — | ~10 |

~21 handler tests remain, ~7 validator tests remain (`app` has 0, `chat_entry` has 4 remaining).

### `crates/slices/` — Feature slices (10 total after VSA-2)

| Slice | Element | Intent | Validator |
|-------|---------|--------|-----------|
| `nsslice-char-counter` | ✅ | — | — |
| `nsslice-status-bar` | ✅ | — | — |
| `nsslice-dashboard` | ✅ | ✅ (4 intents) | — (all infallible) |
| `nsslice-chat-log` | ✅ | — | — |
| `nsslice-provider` | ✅ (2) | — | — |
| `nsslice-pinned-panel` | ✅ | ✅ (11 intents) | ✅ (5 validators) |
| `nsslice-chat-input-box` | ✅ | ✅ (13 intents) | ✅ (3 validators) |
| `nsslice-picker` | — | ✅ (9 intents) | ✅ (9 validators) |
| `nsslice-chat-entry-selection` | — | ✅ (3 intents) | ✅ (3 validators) |

### Future extraction candidates (not in this plan)

- `nsslice-navigation` — scroll, switch-tab, edit-input (8 intents)
- Picker render functions in `nullslop-tui/render.rs` (per plan decision, render dispatch stays centralized)
- `RefreshModels` / `RescanPromptTemplates` / `SessionNew` — move to appropriate slices later
