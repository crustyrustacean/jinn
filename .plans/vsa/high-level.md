# VSA Migration: High-Level Plan

## Problem

The codebase is organized by **technical layer**: state/rendering in `nullslop-component`, intent handling in `nullslop-intent`, keymaps and render dispatch in `nullslop-tui`, protocol types in `nullslop-protocol`. To work on a feature like "pinned panel", a developer must visit 5+ files across 4 crates. Vertical Slice Architecture (VSA) co-locates everything about a feature into one crate so that working on a feature means working in one place.

## Goal

Maximize for **locating code**. Each feature slice is a crate under `crates/slices/` containing its rendering, state (where possible), intent handling, and validators. The slice exposes a `register()` function for wiring. Manual wiring is acceptable — no runtime or compile-time activation/deactivation. Registration is static, called from `src/app.rs`.

## Decisions Made During Planning

### State stays in `nullslop-component`

`AppState` owns state types directly (e.g., `FrontendState.pinned_panel: PinnedPanelState`). If a state type moves to its slice crate, `nullslop-component` would depend on the slice, and the slice depends on `nullslop-component` — circular dependency. State structs remain in `nullslop-component`. The slice imports them. This matches the pattern already established by `nsslice-dashboard` (imports `DashboardState` from `nullslop-component`).

### Intent handling: function-call convention, not trait registry

Each intent-bearing slice exports public handler functions (e.g., `pub fn handle_unpin(state: &mut AppState) -> IntentResult`). The central `IntentHandler::handle()` match block in `nullslop-intent` calls into them. This preserves Rust's exhaustive match checking without introducing trait objects or registries.

### `IntentResult` moves to `nullslop-protocol`

`IntentResult` is a simple struct with `commands: Vec<Command>`. It needs to be accessible to slice crates. Moving it to `nullslop-protocol` (which slices already depend on) avoids adding a dependency on `nullslop-intent`.

### Keymap bindings stay centralized

Keymap bindings are small (~10 lines per scope) and are defined using types from `ratatui-which-key` that live in `nullslop-tui`. Moving them to slice crates would create circular dependencies. The keymap stays in `nullslop-tui` but is reorganized with per-scope sections and comments pointing to the owning slice.

### Render dispatch stays centralized

Overlay rendering (pickers, autocomplete, which-key) is cross-cutting — a picker overlay depends on the picker's entries type + the selection widget. This stays in `nullslop-tui/render.rs`. UiElement dispatch (by name) naturally works with the registration pattern already established.

### Established slice crate pattern

From the 3 already-extracted slices (`nsslice-char-counter`, `nsslice-dashboard`, `nsslice-status-bar`):

```
nsslice-<feature>/
├── Cargo.toml          # depends on nullslop-component-ui, nullslop-component, ratatui
├── src/
│   ├── lib.rs          # pub mod element; register(&mut AppUiRegistry)
│   └── element.rs      # UiElement<AppState> impl + tests
```

For intent-bearing slices, this extends to:

```
nsslice-<feature>/
├── Cargo.toml
├── src/
│   ├── lib.rs          # register(), re-exports
│   ├── element.rs      # UiElement impl + tests
│   ├── intent.rs       # handler functions + tests
│   └── validator.rs    # validators + error types + tests
```

State files stay in `nullslop-component` (see above).

### Cross-cutting handlers stay in `nullslop-intent`

Handlers that touch multiple features (e.g., `handle_set_mode` cancels streams, `handle_interrupt` resets buffer + cancels stream) are split: input-specific parts move to the slice, cross-cutting orchestration stays in `nullslop-intent` and calls into the slice.

---

## Acceptance Criteria

1. No duplicated code — old element/validator/handler files are deleted from their original locations
2. All extracted slices follow the established pattern: `register()`, `element.rs`, optional `intent.rs` + `validator.rs`
3. `cargo test` passes across the whole workspace
4. `nullslop-intent` handler match block calls into slice functions for pinned-panel and chat-input intents
5. State structs remain in `nullslop-component` (no circular deps)
6. Each slice crate is independently testable (`cargo test -p nsslice-pinned-panel`)

---

## Implementation Phases

- [x] ~~Phase 1: Cleanup — Remove orphaned code from `nullslop-component`~~ (merged into Phase 1 execution below)
  - [x] ~~Delete `nullslop-component/src/char_counter/element.rs` (moved to `nsslice-char-counter`)~~
  - [x] ~~Delete `nullslop-component/src/status_bar/element.rs` (moved to `nsslice-status-bar`)~~
  - [x] ~~Delete `nullslop-component/src/dashboard/element.rs` (moved to `nsslice-dashboard`)~~
  - [x] ~~Remove `char_counter` module entirely from `nullslop-component/src/lib.rs` and delete `nullslop-component/src/char_counter/` directory (mod.rs only had element re-export, now empty)~~
  - [x] ~~Remove `status_bar` module entirely from `nullslop-component/src/lib.rs` and delete `nullslop-component/src/status_bar/` directory (mod.rs only had element re-export, now empty)~~
  - [x] ~~Update `nullslop-component/src/dashboard/mod.rs` — remove `element` submodule declaration and `DashboardElement` re-export, keep `state` submodule and `DashboardState` re-export~~
  - [x] ~~Verify `register_tui_elements()` in `nullslop-component/src/lib.rs` no longer registers these 3 elements (already confirmed: they're registered from slices in `src/app.rs`)~~
  - [x] ~~Run `cargo test` to confirm no breakage~~

- [x] ~~Phase 2: Extract `nsslice-chat-log`~~ (merged into Phase 1 execution below)
  - [x] ~~Create `crates/slices/nsslice-chat-log/Cargo.toml` — depends on `nullslop-component-ui`, `nullslop-component`, `nullslop-protocol` (for `ChatEntryKind`), `ratatui`; dev-dep on `rstest`~~
  - [x] ~~Create `crates/slices/nsslice-chat-log/src/lib.rs` — `pub mod element; pub use element::ChatLogElement;` + `pub fn register(registry: &mut AppUiRegistry)`~~
  - [x] ~~Create `crates/slices/nsslice-chat-log/src/element.rs` — moved from `nullslop-component/src/chat_log/element.rs`, update imports from `crate::` to `nullslop_component::`~~
  - [x] ~~Move all tests from the old element to the new one~~
  - [x] ~~Delete `nullslop-component/src/chat_log/element.rs`~~
  - [x] ~~Update `nullslop-component/src/chat_log/mod.rs` — remove element submodule, remove `ChatLogElement` re-export; if module is now empty, delete directory and remove from `lib.rs`~~
  - [x] ~~Remove `chat_log` from `nullslop-component/src/lib.rs` if module becomes empty~~
  - [x] ~~Remove `ChatLogElement` registration from `register_tui_elements()` in `nullslop-component/src/lib.rs`~~
  - [x] ~~Add `nsslice-chat-log` to workspace members and dependencies in root `Cargo.toml` (follow the pattern: `crates/slices` is excluded from the glob, individual slices are listed explicitly)~~
  - [x] ~~Add `nsslice_chat_log::register(&mut ui_registry)` in `src/app.rs` (both TUI and headless registration paths, ~lines 140 and 508)~~
  - [x] ~~Run `cargo test`~~

> **Phase 1+2 execution notes**: Original Phase 1 (cleanup) and Phase 2 (chat-log extraction) were merged into a single `.plans/vsa/phase-1.md` and executed together. An **unplanned** change was required: `nullslop-tui/Cargo.toml` needed slice dependencies added, and 3 registration sites in `crates/nullslop-tui/src/app.rs` (`TuiAppBuilder::build()` and two other builder paths) needed `nsslice_*::register()` calls. This is because the TUI tests build their own `AppUiRegistry` and the chat-log element was previously registered by `nullslop_component::register_all()`. **Future phases must also update `nullslop-tui`** — add the new slice dependency to `crates/nullslop-tui/Cargo.toml` and add `register()` calls at all 3 `nullslop_component::register_all()` call sites in `crates/nullslop-tui/src/app.rs`.

- [x] ~~Phase 3: Extract `nsslice-provider`~~
  - [x] ~~Create `crates/slices/nsslice-provider/Cargo.toml` — depends on `nullslop-component-ui`, `nullslop-component`, `ratatui`, `throbber-widgets-tui`, `unicode-segmentation`; dev-dep on `rstest`~~
  - [x] ~~Create `crates/slices/nsslice-provider/src/lib.rs` — `pub mod indicator; pub mod queue_element;` + `pub fn register(registry: &mut AppUiRegistry)` that registers both `StreamingIndicatorElement` and `QueueDisplayElement`~~
  - [x] ~~Create `crates/slices/nsslice-provider/src/indicator.rs` — moved from `nullslop-component/src/provider/indicator.rs`, update imports~~
  - [x] ~~Create `crates/slices/nsslice-provider/src/queue_element.rs` — moved from `nullslop-component/src/provider/queue_element.rs`, update imports~~
  - [x] ~~Move all tests with their elements~~
  - [x] ~~Delete `nullslop-component/src/provider/` directory entirely (mod.rs, indicator.rs, queue_element.rs)~~
  - [x] ~~Remove `provider` module from `nullslop-component/src/lib.rs`~~
  - [x] ~~Remove `StreamingIndicatorElement` and `QueueDisplayElement` from `register_tui_elements()`~~
  - [x] ~~Add workspace entries in root `Cargo.toml`~~
  - [x] ~~Add `nsslice_provider::register(&mut ui_registry)` in `src/app.rs`~~
  - [x] ~~Add `nsslice-provider` dependency to `crates/nullslop-tui/Cargo.toml`, add `nsslice_provider::register()` at all 3 `register_all()` sites in `crates/nullslop-tui/src/app.rs` (see Phase 1+2 notes)~~
  - [x] ~~Run `cargo test`~~

- [x] ~~Phase 4: Introduce intent registration convention~~
  - [x] ~~Move `IntentResult` from `nullslop-intent/src/handler.rs` to `nullslop-protocol` (new file `nullslop-protocol/src/intent_result.rs`)~~
  - [x] ~~Re-export `IntentResult` from `nullslop-intent` for backward compatibility: `pub use nullslop_protocol::IntentResult;`~~
  - [x] ~~No external `use` statements needed updating — `IntentResult` had no consumers outside `nullslop-intent`~~
  - [x] ~~Document the intent-bearing slice convention in `nsslice-chat-log/src/lib.rs` doc comments~~
  - [x] ~~Run `cargo test`~~

- [x] ~~Phase 5: Extract `nsslice-pinned-panel` (first intent-bearing slice)~~
  - [x] ~~Create `crates/slices/nsslice-pinned-panel/Cargo.toml` — depends on `nullslop-component-ui`, `nullslop-component`, `nullslop-protocol`, `ratatui`, `wherror`; dev-dep on `rstest`~~
  - [x] ~~Create `crates/slices/nsslice-pinned-panel/src/lib.rs` — `pub mod element; pub mod intent; pub mod validator;` + `register()`~~
  - [x] ~~Create `crates/slices/nsslice-pinned-panel/src/element.rs` — moved from `nullslop-component/src/pinned_panel/element.rs`, update imports~~
  - [x] ~~Create `crates/slices/nsslice-pinned-panel/src/validator.rs` — moved from `nullslop-intent/src/validators/pinned_panel.rs`~~
  - [x] ~~Create `crates/slices/nsslice-pinned-panel/src/intent.rs` — handler functions + 13 tests moved from `nullslop-intent`~~
  - [x] ~~Update `nullslop-component/src/pinned_panel/mod.rs` — remove element submodule~~
  - [x] ~~Remove `PinnedPanelElement` from `register_tui_elements()`~~
  - [x] ~~Update `nullslop-intent/src/handler.rs` — delegate 11 match arms to slice, remove 5 handler functions + 2 helpers~~
  - [x] ~~Remove `pinned_panel` from `nullslop-intent/src/validators/mod.rs`~~
  - [x] ~~Delete `nullslop-intent/src/validators/pinned_panel.rs`~~
  - [x] ~~Delete `nullslop-component/src/pinned_panel/element.rs`~~
  - [x] ~~Add workspace entries in root `Cargo.toml`~~
  - [x] ~~Add registration at all 5 sites (2 in `src/app.rs`, 3 in `nullslop-tui/src/app.rs`)~~
  - [x] ~~Add `nsslice-pinned-panel` dependency to `nullslop-intent` and `nullslop-tui` Cargo.toml~~
  - [x] ~~Run `cargo test --workspace` — all pass~~

> **Phase 5 execution notes**: Implemented as planned with minor divergences: (1) Test count is 13 handler tests (not 14) — the plan listed `pinned_panel_pin_cycle_returns_command` which doesn't exist; the actual tests are `pinned_panel_pin_cycle_rotates_top_to_bottom`. (2) `UnpinChatEntry` import removed from handler.rs since it became unused after the handler moved. (3) Tests call slice functions directly instead of going through `IntentHandler::handle()`.

- [x] ~~Phase 6: Extract `nsslice-chat-input-box`~~
  - [x] ~~Create `crates/slices/nsslice-chat-input-box/Cargo.toml` — depends on `nullslop-component-ui`, `nullslop-component`, `nullslop-protocol`, `ratatui`, `unicode-segmentation`; dev-dep on `rstest`~~
  - [x] ~~Create `crates/slices/nsslice-chat-input-box/src/lib.rs`~~
  - [x] ~~Create `crates/slices/nsslice-chat-input-box/src/element.rs` — moved from `nullslop-component/src/chat_input_box/element.rs`, update imports~~
  - [x] ~~Create `crates/slices/nsslice-chat-input-box/src/validator.rs` — moved from `nullslop-intent/src/validators/chat_input.rs`~~
  - [x] ~~Create `crates/slices/nsslice-chat-input-box/src/intent.rs` — 5 handler functions + 8 thin cursor wrappers + 3 helpers + 14 tests~~
  - [x] ~~**State stays**: `ChatInputBoxState` remains in `nullslop-component/src/chat_input_box/state.rs`~~
  - [x] ~~Update `nullslop-component/src/chat_input_box/mod.rs` — remove `element` submodule~~
  - [x] ~~Remove `ChatInputBoxElement` from `register_tui_elements()` in `nullslop-component/src/lib.rs`~~
  - [x] ~~Update `nullslop-intent/src/handler.rs` — delegate 13 match arms to slice, remove handlers/helpers, update interrupt validator call~~
  - [x] ~~Remove `chat_input` from `nullslop-intent/src/validators/mod.rs`~~
  - [x] ~~Delete `nullslop-intent/src/validators/chat_input.rs`~~
  - [x] ~~Move 14 chat-input handler tests from `nullslop-intent/src/handler_tests.rs` to slice~~
  - [x] ~~Delete `nullslop-component/src/chat_input_box/element.rs`~~
  - [x] ~~Add workspace entries in root `Cargo.toml`~~
  - [x] ~~Add `nsslice_chat_input_box::register(&mut ui_registry)` at all 5 sites (2 in `src/app.rs`, 3 in `nullslop-tui/src/app.rs`)~~
  - [x] ~~Add `nullslop-intent` and `nullslop-tui` dependencies on `nsslice-chat-input-box`~~
  - [x] ~~Remove `unicode-segmentation` from `nullslop-intent/Cargo.toml` (now only used by slice)~~
  - [x] ~~Run `cargo test` — all pass~~

> **Phase 6 execution notes**: Implemented as planned per the detailed `phase-6.md` plan. Key decisions: (1) `handle_interrupt` and `handle_set_mode` stay in `nullslop-intent` — cross-cutting handlers that call into the slice's validator. Only `handle_interrupt`'s validator call changed to `nsslice_chat_input_box::validator::validate_interrupt(state)`. (2) The 4 interrupt handler tests stay in `nullslop-intent` since they test cross-cutting behavior. (3) 8 inline cursor movement match arms became thin wrapper functions in the slice (`handle_move_cursor_left`, etc.). (4) `register_tui_elements()` in `nullslop-component` is now empty (all elements registered by slices). (5) `unicode-segmentation` removed from `nullslop-intent` since only `handle_insert_char` used it. (6) The lib.rs does NOT re-export `ChatInputBoxState` (not needed — consumers import from `nullslop_component`). Test counts: 33 in slice (11 element + 8 validator + 14 intent), 64 in intent (was ~78, minus 14 moved).

---

## Post-Phase-6 State of the Codebase

### `nullslop-component` — State + types only

| Module | Contents |
|--------|----------|
| `app_state.rs` | `AppState`, `FrontendState`, `SessionState`, `ContextAssemblyState`, `ProviderState`, `ShutdownCoordinatorState` |
| `state.rs` | `State` (RwLock wrapper), `StateReadGuard`, `StateWriteGuard` |
| `tui_signals.rs` | `TuiSignals` |
| `picker_highlight.rs` | `PICKER_HIGHLIGHT_STYLE` |
| `chat_input_box/` | `ChatInputBoxState` (state + state tests only) |
| `chat_session/` | `ChatSessionState` (unchanged) |
| `pinned_panel/` | `PinnedPanelState` (state only) |
| `dashboard/` | `DashboardState` (state only) |
| `context_strategy_picker/` | `StrategyEntry` (entries + tests) |
| `keymap_picker/` | `KeymapEntry` (entries + tests) |
| `provider_picker/` | `PickerEntry` (entries + tests + loader) |
| `session_picker/` | `SessionEntry` (entries + tests) |
| `prompt_template/` | `PromptTemplateStore` |
| `shutdown_tracker/` | `ShutdownTrackerState` |
| `app_quit/` | empty shell |
| `chat_entry_selection/` | empty shell |
| `context_pin/` | empty shell |
| `tab_nav/` | empty shell |

### `crates/slices/` — Feature slices

| Slice | Element | State | Intent | Validator |
|-------|---------|-------|--------|-----------|
| `nsslice-char-counter` | ✅ | — | — | — |
| `nsslice-status-bar` | ✅ | — | — | — |
| `nsslice-dashboard` | ✅ | — | — | — |
| `nsslice-chat-log` | ✅ | — | — | — |
| `nsslice-provider` | ✅ (2 sub-elements) | — | — | — |
| `nsslice-pinned-panel` | ✅ | stays in component | ✅ (11 intents) | ✅ (5 validators) |
| `nsslice-chat-input-box` | ✅ | stays in component | ✅ (13 intents) | ✅ (3 validators) |

### `nullslop-intent` — Central dispatch + remaining handlers

After Phase 6, the handler match block becomes thinner:
- Calls `nsslice_chat_input_box::intent::*()` for chat-input intents
- Calls `nsslice_pinned_panel::intent::*()` for pinned-panel intents
- Directly handles: navigation, mode/app, picker, dashboard, chat-entry intents (until those are extracted too)
- Validators remaining: `app`, `chat_entry`, `picker` (dashboard validator is empty)
