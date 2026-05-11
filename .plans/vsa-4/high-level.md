# VSA-4: Detour — Split SetMode, Add Interrupt Session Targeting, Move NormalEscape

## Problem

After VSA-3, `nullslop-intent/src/handler.rs` still contains 2 inline handlers (`handle_set_mode`, `handle_normal_escape`) and 3 delegations to `nsslice-global`. VSA-4 completes the extraction by:

1. **Splitting `SetMode { mode }` into `EnterInsertMode` + `EnterNormalMode`** — two focused intents instead of one generic mode setter with conditional branching
2. **Adding `session_id: Option<SessionId>` to `Interrupt`** — enables programmatic cancel of any session, not just the active one
3. **Moving `NormalEscape` to `nsslice-chat-input-box`** — it clears chat entry selection, which is the chat input box's domain
4. **Fixing the `pinned_pane_close` bug** — `NormalEscape` incorrectly sets the pinned pane close signal; pinned panel has its own close intent
5. **Extracting `cancel_stream_and_drain` to a `ChatSessionState` method** — eliminates the shared helper in `nsslice-global` that `nsslice-chat-input-box` can't reach (no cross-slice deps)

After VSA-4, `nullslop-intent` is a pure dispatch hub: `tui_signals.clear()` + exhaustive `match intent { ... }` where every arm delegates to a slice crate, plus the picker re-dispatch logic for keymap confirm.

## Decisions

### `Interrupt { session_id: Option<SessionId> }` — two paths

Discussed with user. `Interrupt`'s purpose is canceling a stream.

- **`session_id: None`** — current "smart" behavior on the active session: validate, deactivate autocomplete, cancel stream or reset input buffer, drain queued messages
- **`session_id: Some(id)`** — targeted cancel: just call `cancel_streaming()` on that session and emit `CancelStream`. No validation, no autocomplete deactivation, no queue drain. This enables the actor system to cancel any session programmatically.

The `None` path preserves exact current behavior. The `Some` path is the new "cancel a specific stream" API.

### `EnterInsertMode` / `EnterNormalMode` live in `nsslice-chat-input-box`

User confirmed: insert mode IS the chat input box. Normal mode IS "no longer in insert mode." Both handlers belong there.

- **`EnterInsertMode`** — trivial: just set `mode = Input`
- **`EnterNormalMode`** — cancel stream if in Input mode + streaming, clear `active_picker_kind` if in Picker mode, set `mode = Normal`. The picker cleanup is part of "return to neutral," not picker-specific logic.

### `cancel_stream_and_drain` → method on `ChatSessionState`

Shared helper was in `nsslice-global`. Moving to `nsslice-chat-input-box` would create a cross-slice dependency. User specified: "target a specific session, don't poke around inside AppState." Solution: make it a method on `ChatSessionState` in `nullslop-component` (both slices already depend on it).

```rust
// On ChatSessionState in nullslop-component/src/chat_session/state.rs
pub fn cancel_stream_and_drain(&mut self) {
    self.cancel_streaming();
    let drained: Vec<String> = self.drain_queue().into_iter().collect();
    let drained_text = drained.join("\n");
    if !drained_text.is_empty() {
        self.chat_input_mut().replace_all(drained_text);
    }
}
```

Callers are explicit about which session:
- `nsslice-global`: `state.active_session_mut().cancel_stream_and_drain()`
- `nsslice-chat-input-box`: `state.active_session_mut().cancel_stream_and_drain()`
- `nsslice-global` (targeted): `state.session_mut(&id).cancel_streaming()`

### `NormalEscape` pinned_pane_close bug fix

Current handler incorrectly sets `state.frontend.tui_signals.pinned_pane_close = true`. The pinned panel has its own close intent (`PinnedPanelClose`). This bug is fixed during the move — the `pinned_pane_close` line is dropped.

The test `normal_escape_sets_close_signal_even_without_selection` becomes meaningless after the fix (it would just test "no commands, no state change"). **Delete this test.** Keep `normal_escape_clears_selection` (moves to `nsslice-chat-input-box`, minus the `pinned_pane_close` assertion).

### `validate_normal_escape` moves to `nsslice-chat-input-box`

After VSA-3, it was in `nsslice-global/src/validator.rs`. The handler moves to `nsslice-chat-input-box`. Colocate the validator with its handler. Remove from `nsslice-global`.

### Keymap and TUI updates are mechanical

All references to `Intent::SetMode { mode: Mode::Input }` become `Intent::EnterInsertMode`. All `Intent::SetMode { mode: Mode::Normal }` become `Intent::EnterNormalMode`. All `Intent::Interrupt` (unit variant) become `Intent::Interrupt { session_id: None }`. The `app.rs` pattern match updates from `Intent::SetMode { .. }` to `Intent::EnterNormalMode`. The `EnterInsertMode` path should NOT cancel mouse selection.

### `tui_signals_are_cleared_at_start_of_handle` test

Stays unchanged. Uses `Intent::Quit` as trigger, tests the IntentHandler preamble. Still works through delegation.

---

## Acceptance Criteria

1. `Intent::SetMode` no longer exists — replaced by `EnterInsertMode` and `EnterNormalMode`
2. `Intent::Interrupt` has field `session_id: Option<SessionId>` — `None` preserves current behavior, `Some(id)` does targeted cancel
3. `cancel_stream_and_drain` is a method on `ChatSessionState` in `nullslop-component`, no longer in `nsslice-global`
4. `NormalEscape` handler lives in `nsslice-chat-input-box`, no longer sets `pinned_pane_close`
5. `validate_normal_escape` lives in `nsslice-chat-input-box`, no longer in `nsslice-global`
6. `nullslop-intent/src/handler.rs` has no inline handler logic — all arms delegate to slice crates
7. `nullslop-intent/src/handler.rs` no longer imports `CancelStream`, `Command`, or `Mode`
8. Keymap binds `EnterInsertMode`/`EnterNormalMode` instead of `SetMode`
9. `app.rs` cancels selection on `EnterNormalMode` (not `EnterInsertMode`)
10. `cargo test --workspace` passes — no regressions

---

## Implementation Phases

- [x] Phase 1: Split SetMode, target Interrupt, move NormalEscape
  - [x] **1.1** Add `cancel_stream_and_drain(&mut self)` method to `ChatSessionState` in `crates/nullslop-component/src/chat_session/state.rs` — extract logic from `nsslice-global/src/intent.rs`, all operations are on `self` (cancel_streaming, drain_queue, chat_input_mut, replace_all). Add a test.
  - [x] **1.2** Update `Intent` enum in `crates/nullslop-protocol/src/intent.rs`:
    - Replace `SetMode { mode: Mode }` with `EnterInsertMode` and `EnterNormalMode` (remove `Mode` import if no longer needed)
    - Change `Interrupt` to `Interrupt { session_id: Option<SessionId> }` (add `use crate::session::SessionId;`)
    - Update `Display` impl: `Interrupt { .. } => write!(f, "interrupt")`, `EnterInsertMode => write!(f, "enter insert mode")`, `EnterNormalMode => write!(f, "enter normal mode")`
    - Update `NormalEscape` doc comment: remove "close pinned panel" reference
  - [x] **1.3** Update `crates/slices/nsslice-global/src/validator.rs` — remove `validate_normal_escape` function and its doc comment
  - [x] **1.4** Update `crates/slices/nsslice-global/src/intent.rs`:
    - Change `handle_interrupt(state: &mut AppState)` to `handle_interrupt(state: &mut AppState, target: Option<&SessionId>)`
    - Add `use nullslop_protocol::session::SessionId;`
    - `Some(id)` path: `state.session_mut(id).cancel_streaming()`, emit `CancelStream { session_id: id.clone() }`
    - `None` path: current logic but call `state.active_session_mut().cancel_stream_and_drain()` instead of the removed free function
    - Remove `pub fn cancel_stream_and_drain(state: &mut AppState)` entirely
    - Update test helper: `fn handle_interrupt(state) { super::handle_interrupt(state, None) }`
    - Add test `interrupt_with_specific_session_cancels_stream` — creates a second session, starts streaming, calls `handle_interrupt(state, Some(&id))`, verifies stream cancelled and CancelStream emitted
  - [x] **1.5** Update `crates/slices/nsslice-chat-input-box/src/validator.rs` — add `validate_normal_escape(_state: &AppState) {}` with doc comment, at the end of the validators (after `validate_autocomplete_confirm`)
  - [x] **1.6** Update `crates/slices/nsslice-chat-input-box/src/intent.rs`:
    - Add imports: `use nullslop_protocol::provider::CancelStream;` and add `Mode` to existing `nullslop_protocol::{Command, IntentResult}` import
    - Add `handle_enter_insert_mode(state)`: just `state.frontend.mode = Mode::Input; IntentResult::empty()`
    - Add `handle_enter_normal_mode(state)`: cancel stream if Input+streaming (using `state.active_session_mut().cancel_stream_and_drain()`), clear `active_picker_kind` if Picker, set `mode = Normal`
    - Add `handle_normal_escape(state)`: call `crate::validator::validate_normal_escape(state)`, clear selection if present, NO `pinned_pane_close` signal
    - Add 6 tests in the test module:
      - `enter_insert_mode_sets_mode_to_input`
      - `enter_normal_mode_sets_mode_to_normal`
      - `enter_normal_mode_clears_picker_kind_when_leaving_picker`
      - `enter_normal_mode_cancels_stream_when_in_input_mode`
      - `enter_normal_mode_drains_queue_when_cancelling_stream`
      - `normal_escape_clears_selection` (no `pinned_pane_close` assertion)
  - [x] **1.7** Update `crates/slices/nsslice-chat-input-box/src/lib.rs` — remove the note about `handle_interrupt` and `handle_set_mode` staying in `nullslop-intent`; update doc to say 16 intents (13 original + EnterInsertMode + EnterNormalMode + NormalEscape)
  - [x] **1.8** Update `crates/nullslop-intent/src/handler.rs`:
    - Replace `Intent::SetMode { mode } => handle_set_mode(state, *mode)` with `Intent::EnterInsertMode => nsslice_chat_input_box::intent::handle_enter_insert_mode(state)` and `Intent::EnterNormalMode => nsslice_chat_input_box::intent::handle_enter_normal_mode(state)`
    - Replace `Intent::Interrupt => nsslice_global::intent::handle_interrupt(state)` with `Intent::Interrupt { session_id } => nsslice_global::intent::handle_interrupt(state, session_id.as_ref())`
    - Replace `Intent::NormalEscape => handle_normal_escape(state)` with `Intent::NormalEscape => nsslice_chat_input_box::intent::handle_normal_escape(state)`
    - Remove `handle_set_mode` function entirely
    - Remove `handle_normal_escape` function entirely
    - Remove `// --- Chat input handlers ---` and `// --- Mode & App handlers ---` comments (empty sections)
    - Clean imports: remove `use nullslop_protocol::provider::CancelStream;`, remove `Command` and `Mode` from `use nullslop_protocol::{Command, Mode, PinPosition};` → `use nullslop_protocol::PinPosition;`
  - [x] **1.9** Update `crates/nullslop-intent/src/handler_tests.rs` — remove 6 tests:
    - `set_mode_changes_mode`
    - `set_mode_clears_picker_kind_when_leaving_picker`
    - `set_mode_input_to_normal_during_streaming_cancels_stream`
    - `set_mode_input_to_normal_during_streaming_drains_queue`
    - `normal_escape_clears_selection`
    - `normal_escape_sets_close_signal_even_without_selection`
    - Clean imports: remove `ChatEntry`, `Command` from imports (no longer used by remaining tests). `Mode`, `PickerKind`, `KeymapEntry`, `AppState`, `FrontendState` remain used.
  - [x] **1.10** Update `crates/nullslop-tui/src/keymap.rs`:
    - `Intent::SetMode { mode: Mode::Input }` → `Intent::EnterInsertMode` (2 occurrences: Normal scope `i`, Pinned scope `i`)
    - `Intent::SetMode { mode: Mode::Normal }` → `Intent::EnterNormalMode` (2 occurrences: Input scope `<esc>`, Picker scope `<esc>`)
    - `Intent::Interrupt` → `Intent::Interrupt { session_id: None }` (1 occurrence: Input scope `<c-c>`)
    - Remove `Mode` from imports if no longer used
  - [x] **1.11** Update `crates/nullslop-tui/src/app.rs` — change `matches!(intent, Intent::SetMode { .. } | Intent::NormalEscape)` to `matches!(intent, Intent::EnterNormalMode | Intent::NormalEscape)`. Note: `EnterInsertMode` should NOT cancel selection.
  - [x] **1.12** Update `crates/nullslop-tui/src/render_tests.rs` — change `Intent::SetMode { mode: Mode::Normal }` to `Intent::EnterNormalMode`
  - [x] **1.13** Run `cargo test --workspace`

---

## Post-VSA-4 State

### `nullslop-intent/src/handler.rs`

The handler is now a pure dispatch hub. Every match arm delegates:

```rust
match intent {
    // --- Chat Input (13) → nsslice-chat-input-box ---
    // --- Navigation (8) → nsslice-navigation ---
    // --- Global (3) → nsslice-global ---
    // --- Mode (2) → nsslice-chat-input-box ---
    // --- NormalEscape (1) → nsslice-chat-input-box ---
    // --- Picker (9) → nsslice-picker ---
    // --- Session (3) → nsslice-session-management ---
    // --- Dashboard (4) → nsslice-dashboard ---
    // --- Pinned Panel (11) → nsslice-pinned-panel ---
    // --- Chat Entry Selection (3) → nsslice-chat-entry-selection ---
}
```

Remaining non-delegation logic:
- `state.frontend.tui_signals.clear()` preamble
- Picker re-dispatch: `PickerConfirm` → if keymap entry, re-invoke `IntentHandler::handle`

### `crates/slices/` — All feature slices after VSA-4 (12 total)

| Slice | Element | Intent | Validator |
|-------|---------|--------|-----------|
| `nsslice-char-counter` | ✅ | — | — |
| `nsslice-status-bar` | ✅ | — | — |
| `nsslice-dashboard` | ✅ | ✅ (4) | — |
| `nsslice-chat-log` | ✅ | — | — |
| `nsslice-provider` | ✅ | — | — |
| `nsslice-pinned-panel` | ✅ | ✅ (11) | ✅ (5) |
| `nsslice-chat-input-box` | ✅ | ✅ (16) | ✅ (3) |
| `nsslice-picker` | — | ✅ (9) | ✅ (9) |
| `nsslice-chat-entry-selection` | — | ✅ (3) | ✅ (3) |
| `nsslice-navigation` | — | ✅ (8) | — |
| `nsslice-session-management` | — | ✅ (3) | ✅ (3) |
| `nsslice-global` | — | ✅ (3) | ✅ (3) |

### Intent count change

| Before VSA-4 | After VSA-4 |
|---|---|
| `SetMode { mode }` (1 variant) | `EnterInsertMode` + `EnterNormalMode` (2 variants) |
| `Interrupt` (no fields) | `Interrupt { session_id: Option<SessionId> }` |
| 54 total intents | 55 total intents |
