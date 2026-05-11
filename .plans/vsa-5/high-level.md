# VSA-5: Dissolve Picker Entries, Render Functions, Prompt Template out of `nullslop-component`

## Problem

After VSA-1 through VSA-4, all intent handlers and validators have been extracted into slice crates. `nullslop-intent` is a pure dispatch hub. However, `nullslop-component` still contains ~7,235 lines of code that belongs to specific features: picker entry types (`PickerEntry`, `SessionEntry`, `KeymapEntry`, `StrategyEntry`), their `PickerItem` impls, loaders, render helpers, and extensive tests. Additionally, `nullslop-tui/render.rs` contains per-feature picker render functions and the autocomplete popup render that logically belong in their owning slices.

The goal of VSA is that working on a feature means working in one place. Currently, to work on "provider picker", you visit `nullslop-component/provider_picker/` (entries), `nsslice-provider/` (element), `nsslice-picker/` (intent+validator), and `nullslop-tui/render.rs` (rendering). After VSA-5, you visit `nsslice-provider/` (element + entries + render) and `nsslice-picker/` (intent+validator).

The ultimate goal is for `nullslop-component` to cease to exist. VSA-5 dissolves as much of it as possible without refactoring `AppState`.

## Decisions

### Entry struct definitions move to `nullslop-protocol`

Entry types (`PickerEntry`, `SessionEntry`, `KeymapEntry`, `StrategyEntry`) are referenced from `FrontendState` in `app_state.rs` as `SelectionState<PickerEntry>` etc. The struct definitions must live in a crate that both `nullslop-component` (for AppState) and the slice crates can depend on — `nullslop-protocol` is that crate. All four entry types' fields only use `String`, `bool`, `u64`, `SessionId`, `PromptStrategyId`, `Intent`, and `jiff::Timestamp` — all already available in `nullslop-protocol`.

Only the bare struct definitions move. `PickerItem` impls, render helpers, loaders, and tests stay in the domain slice crates.

### `PICKER_HIGHLIGHT_STYLE` + `highlight_text()` move to `nullslop-selection-widget`

`picker_highlight.rs` (79 lines) provides `highlight_text()` — a fuzzy-match highlight function used by `provider_picker`, `context_strategy_picker`, and should-be-used-by `session_picker` (which currently has a private duplicate). It also defines `PICKER_HIGHLIGHT_STYLE`. Both are picker infrastructure, not feature-specific. Moving to `nullslop-selection-widget` keeps them in a crate all picker-bearing slices already depend on (or can add as a dep).

### `PickerItem` impls + loaders + helpers + tests → domain slices

| Entry Type | `PickerItem` impl + loader + tests → |
|---|---|
| `PickerEntry` | `nsslice-provider` (new `entries.rs` + `entries_tests.rs` + `loader.rs`) |
| `SessionEntry` | `nsslice-session-management` (new `entries.rs`) |
| `KeymapEntry` | `nsslice-picker` (new `entries.rs` + inline tests) |
| `StrategyEntry` | `nsslice-picker` (new `entries.rs` + `entries_tests.rs`) |

### Render functions → domain slices

Each picker render function moves to the slice that owns its entry type. The render functions create a `SelectionWidget` and call `.render()` — they need `nullslop-selection-widget` + `ratatui`, which is fine (widgets are shared code).

| Render function | Moves to |
|---|---|
| `render_provider_picker()` | `nsslice-provider` |
| `render_session_picker()` | `nsslice-session-management` |
| `render_keymap_picker()` | `nsslice-picker` |
| `render_context_strategy_picker()` | `nsslice-picker` |
| `render_autocomplete_popup()` + `scroll_window()` + constants | `nsslice-chat-input-box` |

The `render_picker()` dispatcher in `render.rs` stays in `nullslop-tui` — it reads `active_picker_kind` from `AppState` and delegates. But instead of calling local functions, it calls `nsslice_provider::render::render_provider_picker()`, etc.

### `expand_tokens()` → `nullslop-prompt-template`

Rather than moving `expand_tokens()` to `nsslice-chat-input-box` (which would create a dep on `unicode-segmentation`), it moves to `nullslop-prompt-template` — the standalone crate that already owns `PromptTemplateStore`. It's a pure function over `PromptTemplateStore`, which is the domain of that crate. The `nullslop-component/src/prompt_template/mod.rs` file then becomes just re-exports.

### Empty shell modules: delete

`app_quit/`, `chat_entry_selection/`, `context_pin/`, `tab_nav/` — 4 directories containing only stale doc comments about "Phase 7 re-implementation." Delete them and remove from `lib.rs`.

### Empty protocol shell: delete

`nullslop-protocol/src/provider_picker/` contains only empty files (`command.rs` has no code). Delete it and remove from `lib.rs`.

### What stays in `nullslop-component` after VSA-5

| Module | Lines | Why it stays |
|---|---|---|
| `app_state.rs` | 397 | Central state types — blocked by circular dep |
| `state.rs` | 136 | RwLock wrapper |
| `tui_signals.rs` | 99 | Cross-cutting signals |
| `chat_session/` | 1,799 | State + tests — referenced by AppState |
| `chat_input_box/` | 1,039 | State + tests — referenced by AppState |
| `dashboard/` (state only) | 309 | State — referenced by AppState |
| `pinned_panel/` (state only) | 297 | State — referenced by AppState |
| `shutdown_tracker/` | 70 | Lifecycle state |
| `prompt_template/` (~20 lines) | Just re-exports after `expand_tokens` moves | Convenience re-exports |
| `lib.rs` | ~60 | Reduced |

~3,300 lines remain. Dissolving these requires refactoring `AppState` itself (separate future effort).

### Render test migration

Render tests for picker/autocomplete functions move to the corresponding slices. Tests for cross-cutting features (selection highlight, clipboard, layout) stay in `nullslop-tui/render_tests.rs`.

| Tests | Lines | Destination |
|---|---|---|
| Provider picker renders (`render_provider_picker_*`) | ~150 | `nsslice-provider` |
| Context strategy picker renders (`render_context_strategy_picker_*`) | ~110 | `nsslice-picker` |
| Keymap picker renders (`render_keymap_picker_*`) | ~155 | `nsslice-picker` |
| Autocomplete popup renders (`render_autocomplete_popup_*`) | ~290 | `nsslice-chat-input-box` |
| Selection/clipboard/layout tests | ~370 | Stay in `nullslop-tui` |

---

## Acceptance Criteria

1. `nullslop-component/src/{provider_picker,session_picker,keymap_picker,context_strategy_picker}/` deleted entirely
2. `nullslop-component/src/picker_highlight.rs` deleted — `highlight_text` + `PICKER_HIGHLIGHT_STYLE` live in `nullslop-selection-widget`
3. `nullslop-component/src/{app_quit,chat_entry_selection,context_pin,tab_nav}/` deleted (empty shells)
4. `nullslop-protocol/src/provider_picker/` deleted (empty shell)
5. Entry struct definitions (`PickerEntry`, `SessionEntry`, `KeymapEntry`, `StrategyEntry`) live in `nullslop-protocol`
6. `PickerItem` impls, loaders, render helpers, and tests live in domain slice crates
7. Picker render functions live in domain slice crates; `render.rs` dispatches to them
8. Autocomplete popup render lives in `nsslice-chat-input-box`
9. `expand_tokens()` lives in `nullslop-prompt-template`; `nullslop-component/src/prompt_template/` is re-exports only
10. `session_picker/entries.rs` private `highlight_text` duplicate eliminated (uses shared one)
11. `nullslop-component` shrinks from ~7,235 to ~3,300 lines
12. `cargo test --workspace` passes — no regressions

---

## Implementation Phases

- [ ] Phase 1: Cleanup — delete empty shells + move shared picker infrastructure
  - [ ] 1.1 Delete `nullslop-component/src/app_quit/` directory and remove `pub mod app_quit;` from `lib.rs`
  - [ ] 1.2 Delete `nullslop-component/src/chat_entry_selection/` directory and remove `pub mod chat_entry_selection;` from `lib.rs`
  - [ ] 1.3 Delete `nullslop-component/src/context_pin/` directory and remove `pub mod context_pin;` from `lib.rs`
  - [ ] 1.4 Delete `nullslop-component/src/tab_nav/` directory and remove `pub mod tab_nav;` from `lib.rs`
  - [ ] 1.5 Delete `nullslop-protocol/src/provider_picker/` directory (contains only `mod.rs` + empty `command.rs`), remove `pub mod provider_picker;` from `nullslop-protocol/src/lib.rs`, remove comment `// provider_picker module kept for transition; no types re-exported.`
  - [ ] 1.6 Move `PICKER_HIGHLIGHT_STYLE` from `nullslop-component/src/lib.rs` to `nullslop-selection-widget/src/lib.rs` (or a new `highlight.rs` module). `PICKER_HIGHLIGHT_STYLE` is a `Style` constant — `nullslop-selection-widget` already depends on `ratatui`.
  - [ ] 1.7 Move `highlight_text()` from `nullslop-component/src/picker_highlight.rs` to `nullslop-selection-widget/src/highlight.rs` (or inline in `lib.rs`). Update to use the relocated `PICKER_HIGHLIGHT_STYLE`. Export publicly.
  - [ ] 1.8 Add `nullslop-selection-widget` re-exports: `pub use highlight::{highlight_text, PICKER_HIGHLIGHT_STYLE};`
  - [ ] 1.9 Add `nullslop-selection-widget` as a dependency to any slice that needs `highlight_text` but doesn't already have it (check `nsslice-provider`, `nsslice-session-management`, `nsslice-picker` — most already depend on it for `SelectionState`)
  - [ ] 1.10 In `session_picker/entries.rs`: replace the private `highlight_text` duplicate (~40 lines) with `use nullslop_selection_widget::highlight_text;` (exact same signature). This should happen before the entries move to a slice in Phase 2.
  - [ ] 1.11 Delete `nullslop-component/src/picker_highlight.rs` and remove `pub mod picker_highlight;` from `lib.rs`. Update all imports in `nullslop-component` that referenced `crate::picker_highlight` → `nullslop_selection_widget::highlight_text`.
  - [ ] 1.12 Update `nullslop-component/src/lib.rs` — remove `PICKER_HIGHLIGHT_STYLE` const, remove `use ratatui::style::{Color, Modifier, Style};` if no longer used by `lib.rs` itself.
  - [ ] 1.13 Run `cargo test --workspace`

- [x] Phase 2: Move entry struct definitions to `nullslop-protocol`, impls + loaders + tests to slices ✅

  **Deviation from plan:** The plan called for moving `PickerItem` impls to slice crates, but Rust's orphan rule prevents implementing a foreign trait on a foreign type in a third crate. The `PickerItem` impls + render helpers moved to `nullslop-protocol` alongside the struct definitions. Loaders, sorting, formatting, and tests moved to slices as planned.

  - [x] 2.1 **`PickerEntry` → `nullslop-protocol`**:
    - Created `nullslop-protocol/src/provider_picker/{mod.rs, entries.rs}` with struct + `PickerItem` impl + render helpers.
    - Added `nullslop-selection-widget` + `ratatui` deps to `nullslop-protocol/Cargo.toml`.
    - Re-exported from `nullslop-protocol` crate root.
  - [x] 2.2 **`PickerEntry` loader + sorting + formatting + tests → `nsslice-provider`**:
    - Added deps: `nullslop-protocol`, `nullslop-selection-widget`, `nullslop-services`, `nullslop-providers`, `jiff`, `humantime`.
    - Created `entries.rs` (loaders, sorting, formatting), `entries_tests.rs` (all tests), `loader.rs` (load_provider_picker_items).
  - [x] 2.3 **`SessionEntry` → `nullslop-protocol`**:
    - Created `nullslop-protocol/src/session_picker/{mod.rs, entries.rs}` with struct + `PickerItem` impl + render helper.
  - [x] 2.4 **`SessionEntry` loader + tests → `nsslice-session-management`**:
    - Added deps: `nullslop-selection-widget`, `nullslop-services`, `ratatui`, `jiff`, `tracing`.
    - Created `entries.rs` (loaders, inline tests).
  - [x] 2.5 **`KeymapEntry` → `nullslop-protocol`**:
    - Created `nullslop-protocol/src/keymap_picker/{mod.rs, entries.rs}` with struct + `PickerItem` impl + render helpers.
  - [x] 2.6 **`KeymapEntry` tests → `nsslice-picker`**:
    - Added deps: `nullslop-selection-widget`, `nullslop-services`, `ratatui`.
    - Created `keymap_entries.rs` (14 tests).
  - [x] 2.7 **`StrategyEntry` → `nullslop-protocol`**:
    - Created `nullslop-protocol/src/context_strategy_picker/{mod.rs, entries.rs}` with struct + `PickerItem` impl + render helper.
  - [x] 2.8 **`StrategyEntry` loader + sorting + formatting + tests → `nsslice-picker`**:
    - Created `strategy_entries.rs` (loaders, sorting, formatting), `strategy_entries_tests.rs` (all tests).
  - [x] 2.9 **Updated `nullslop-component/src/app_state.rs`** — all 4 entry types imported from `nullslop_protocol::`.
  - [x] 2.10 **Updated all external consumers** — `nullslop-tui`, `nullslop-intent`, `nsslice-picker`, `nullslop-provider-actor`.
  - [x] 2.11 **Deleted the four module directories** from `nullslop-component`.
  - [x] 2.12 **Updated `nullslop-component/src/lib.rs`** — removed 4 `pub mod` declarations.
  - [x] 2.13 **`cargo test --workspace` passes** — 1,391 tests, 0 failures.

- [ ] Phase 3: Move render functions + `expand_tokens` to slices (detailed plan: `.plans/vsa-5/phase-3.md`)
  - [ ] 3.1 **`expand_tokens()` → `nullslop-prompt-template`**: move function + 13 tests, add `unicode-segmentation` dep, replace with re-export in component
  - [ ] 3.2 **Picker render functions → slices**: `render_provider_picker` → `nsslice-provider`, `render_session_picker` → `nsslice-session-management`, `render_keymap_picker` + `render_context_strategy_picker` → `nsslice-picker`
  - [ ] 3.3 **Autocomplete popup render → `nsslice-chat-input-box`**: move `render_autocomplete_popup`, `scroll_window`, and 5 constants
  - [ ] 3.4 **Update `nullslop-tui/src/render.rs`**: replace function bodies with dispatch calls, remove moved code
  - [ ] 3.5 **Move render tests to slices**: provider tests → `nsslice-provider`, strategy tests → `nsslice-picker`, keymap tests → `nsslice-picker`, autocomplete tests → `nsslice-chat-input-box`
  - [ ] 3.6 Update `nullslop-tui/Cargo.toml` — add `nsslice-session-management` dep
  - [ ] 3.7 Run `cargo test --workspace`

---

## Post-VSA-5 State

### `nullslop-component` — ~3,300 lines (state only)

| Module | Lines | Contents |
|---|---|---|
| `app_state.rs` | 397 | `AppState`, `FrontendState`, `SessionState`, `ContextAssemblyState`, `ProviderState`, `ShutdownCoordinatorState` |
| `state.rs` | 136 | `State` (RwLock wrapper), guards |
| `tui_signals.rs` | 99 | `TuiSignals` |
| `chat_session/` | 1,799 | `ChatSessionState` + tests |
| `chat_input_box/` | 1,039 | `ChatInputBoxState` + tests |
| `dashboard/` | 309 | `DashboardState` |
| `pinned_panel/` | 297 | `PinnedPanelState` |
| `shutdown_tracker/` | 70 | `ShutdownTrackerState` |
| `prompt_template/` | ~20 | Re-exports only |
| `lib.rs` | ~60 | Reduced public interface |

### `crates/slices/` — All feature slices after VSA-5 (12 total)

| Slice | Element | Intent | Validator | Entries | Render |
|---|---|---|---|---|---|
| `nsslice-char-counter` | ✅ | — | — | — | — |
| `nsslice-status-bar` | ✅ | — | — | — | — |
| `nsslice-dashboard` | ✅ | ✅ (4) | — | — | — |
| `nsslice-chat-log` | ✅ | — | — | — | — |
| `nsslice-provider` | ✅ (2) | — | — | ✅ `PickerEntry` | ✅ picker render |
| `nsslice-pinned-panel` | ✅ | ✅ (11) | ✅ (5) | — | — |
| `nsslice-chat-input-box` | ✅ | ✅ (16) | ✅ (3) | — | ✅ autocomplete render |
| `nsslice-picker` | — | ✅ (9) | ✅ (9) | ✅ `KeymapEntry` + `StrategyEntry` | ✅ picker renders (2) |
| `nsslice-chat-entry-selection` | — | ✅ (3) | ✅ (3) | — | — |
| `nsslice-navigation` | — | ✅ (8) | — | — | — |
| `nsslice-session-management` | — | ✅ (3) | ✅ (3) | ✅ `SessionEntry` | ✅ picker render |
| `nsslice-global` | — | ✅ (3) | ✅ (3) | — | — |

### What's needed to fully dissolve `nullslop-component`

The remaining ~3,300 lines are state structs referenced by `AppState`. Dissolving them requires one of:
- **Extract state traits** — define `trait FrontendStateAccess` etc. in `nullslop-protocol`, implement on `AppState`
- **Split `AppState`** — break the monolith into per-feature state sections that slices own
- **Move to ECS or component model** — each feature registers its own state

This is a separate architectural effort beyond VSA-5.
