# VSA-5 Phase 1: Cleanup — Delete Empty Shells + Move Shared Picker Infrastructure

## Context

Phase 1 of VSA-5 is preparatory cleanup. It deletes 5 empty shell directories and relocates shared picker infrastructure (`PICKER_HIGHLIGHT_STYLE` + `highlight_text()`) from `nullslop-component` to `nullslop-selection-widget`, where all picker-bearing slices can access it. This must happen before Phase 2 (entry type extraction) because Phase 2's slice crates will need `highlight_text` and `PICKER_HIGHLIGHT_STYLE` for their `PickerItem` impls.

## Scope

### Step 1: Delete 4 empty shell modules from `nullslop-component`

These 4 directories contain only stale doc comments referencing "Phase 5" and "Phase 7". No code, no tests, no types, no imports from external crates.

| Directory | File | Contents |
|---|---|---|
| `src/app_quit/` | `mod.rs` | 6-line doc comment |
| `src/chat_entry_selection/` | `mod.rs` | 4-line doc comment |
| `src/context_pin/` | `mod.rs` | 3-line doc comment |
| `src/tab_nav/` | `mod.rs` | 4-line doc comment |

For each: delete directory, remove `pub mod <name>;` from `lib.rs`.

### Step 2: Delete empty `provider_picker` shell from `nullslop-protocol`

`nullslop-protocol/src/provider_picker/` contains `mod.rs` (doc comment only) and `command.rs` (doc comment only). No code, no types, no external references (`grep` confirms no crate imports `nullslop_protocol::provider_picker`).

Delete directory, remove `pub mod provider_picker;` from `nullslop-protocol/src/lib.rs`, remove the `// provider_picker module kept for transition; no types re-exported.` comment.

### Step 3: Move `PICKER_HIGHLIGHT_STYLE` + `highlight_text()` to `nullslop-selection-widget`

**Why `nullslop-selection-widget`:** It's the crate that defines `PickerItem`, `SelectionState`, and `SelectionWidget`. The `highlight_text` function operates on picker match ranges and produces `Span`s — it's picker rendering infrastructure. All picker-bearing slices either already depend on this crate or will add it as a dep in Phase 2. No circular dependency risk.

**What moves:**
- `PICKER_HIGHLIGHT_STYLE` constant (currently in `nullslop-component/src/lib.rs`, line 65)
- `highlight_text()` function (currently in `nullslop-component/src/picker_highlight.rs`, 79 lines)

**Target:** New file `nullslop-selection-widget/src/highlight.rs`. Exported via `pub use highlight::{highlight_text, PICKER_HIGHLIGHT_STYLE};` in `lib.rs`.

### Step 4: Eliminate `session_picker` private `highlight_text` duplicate

`nullslop-component/src/session_picker/entries.rs` has a private `highlight_text` function (~45 lines, lines 99–143) that duplicates the shared one with minor formatting differences. Replace with `use nullslop_selection_widget::highlight_text;` and remove the private function.

The `use crate::PICKER_HIGHLIGHT_STYLE;` import on line 12 also becomes unnecessary after this change (the private function was the only user within `session_picker`). Remove it.

### Step 5: Update all consumers within `nullslop-component`

After steps 3–4, update internal imports:

| File | Old import | New import |
|---|---|---|
| `provider_picker/entries.rs` line 13 | `use crate::picker_highlight::highlight_text;` | `use nullslop_selection_widget::highlight_text;` |
| `context_strategy_picker/entries.rs` line 13 | `use crate::picker_highlight::highlight_text;` | `use nullslop_selection_widget::highlight_text;` |
| `keymap_picker/entries.rs` line 13 | `use crate::PICKER_HIGHLIGHT_STYLE;` | `use nullslop_selection_widget::PICKER_HIGHLIGHT_STYLE;` |
| `session_picker/entries.rs` line 12 | `use crate::PICKER_HIGHLIGHT_STYLE;` | Remove entirely (no longer used after step 4) |

Then delete `nullslop-component/src/picker_highlight.rs` and remove `pub mod picker_highlight;` from `lib.rs`.

### Step 6: Clean up `nullslop-component/src/lib.rs`

After removing everything:
- Remove `use ratatui::style::{Color, Modifier, Style};` — only used by `PICKER_HIGHLIGHT_STYLE`
- Remove `pub const PICKER_HIGHLIGHT_STYLE` definition
- Remove `pub mod picker_highlight;`
- Remove `pub mod app_quit;`, `pub mod chat_entry_selection;`, `pub mod context_pin;`, `pub mod tab_nav;`
- Update crate-level doc comment to remove references to deleted modules

---

## Acceptance Criteria

1. `nullslop-component/src/{app_quit,chat_entry_selection,context_pin,tab_nav}/` directories do not exist
2. `nullslop-component/src/picker_highlight.rs` does not exist
3. `nullslop-protocol/src/provider_picker/` directory does not exist
4. `PICKER_HIGHLIGHT_STYLE` is defined in `nullslop-selection-widget/src/highlight.rs`
5. `highlight_text()` is defined in `nullslop-selection-widget/src/highlight.rs`
6. Both are re-exported from `nullslop-selection-widget` crate root
7. `session_picker/entries.rs` no longer has a private `highlight_text` function (uses shared one from `nullslop-selection-widget`)
8. No remaining `use crate::picker_highlight` or `use crate::PICKER_HIGHLIGHT_STYLE` in `nullslop-component`
9. `nullslop-component/src/lib.rs` does not import `ratatui::style` or define `PICKER_HIGHLIGHT_STYLE`
10. `cargo test --workspace` passes — no regressions

---

## Implementation Steps

- [ ] Step 1: Delete 4 empty shell modules from `nullslop-component`
  - [ ] 1.1 Delete `crates/nullslop-component/src/app_quit/` directory
  - [ ] 1.2 Delete `crates/nullslop-component/src/chat_entry_selection/` directory
  - [ ] 1.3 Delete `crates/nullslop-component/src/context_pin/` directory
  - [ ] 1.4 Delete `crates/nullslop-component/src/tab_nav/` directory
  - [ ] 1.5 Update `crates/nullslop-component/src/lib.rs` — remove 4 `pub mod` declarations (`app_quit`, `chat_entry_selection`, `context_pin`, `tab_nav`)

- [ ] Step 2: Delete empty `provider_picker` shell from `nullslop-protocol`
  - [ ] 2.1 Delete `crates/nullslop-protocol/src/provider_picker/` directory
  - [ ] 2.2 Update `crates/nullslop-protocol/src/lib.rs` — remove `pub mod provider_picker;` and the `// provider_picker module kept for transition; no types re-exported.` comment

- [ ] Step 3: Create `highlight.rs` in `nullslop-selection-widget`
  - [ ] 3.1 Create `crates/nullslop-selection-widget/src/highlight.rs` containing:
    - `PICKER_HIGHLIGHT_STYLE` const (copied from `nullslop-component/src/lib.rs`)
    - `highlight_text()` function (copied from `nullslop-component/src/picker_highlight.rs`, update self-reference from `crate::PICKER_HIGHLIGHT_STYLE` to `PICKER_HIGHLIGHT_STYLE` since both are in same file now)
    - Module-level doc comment explaining it's shared picker highlight infrastructure
  - [ ] 3.2 Update `crates/nullslop-selection-widget/src/lib.rs` — add `pub mod highlight;` and `pub use highlight::{highlight_text, PICKER_HIGHLIGHT_STYLE};`

- [ ] Step 4: Eliminate `session_picker` private `highlight_text` duplicate
  - [ ] 4.1 In `crates/nullslop-component/src/session_picker/entries.rs`:
    - Replace `use crate::PICKER_HIGHLIGHT_STYLE;` with `use nullslop_selection_widget::highlight_text;`
    - Replace the call `highlight_text(title, base_style, match_indices)` on line 87 — signature is identical, no change needed to the call itself
    - Delete the private `highlight_text` function (lines 99–143)

- [ ] Step 5: Update remaining consumers within `nullslop-component`
  - [ ] 5.1 In `crates/nullslop-component/src/provider_picker/entries.rs` — change `use crate::picker_highlight::highlight_text;` → `use nullslop_selection_widget::highlight_text;`
  - [ ] 5.2 In `crates/nullslop-component/src/context_strategy_picker/entries.rs` — change `use crate::picker_highlight::highlight_text;` → `use nullslop_selection_widget::highlight_text;`
  - [ ] 5.3 In `crates/nullslop-component/src/keymap_picker/entries.rs` — change `use crate::PICKER_HIGHLIGHT_STYLE;` → `use nullslop_selection_widget::PICKER_HIGHLIGHT_STYLE;`

- [ ] Step 6: Delete `picker_highlight` module from `nullslop-component`
  - [ ] 6.1 Delete `crates/nullslop-component/src/picker_highlight.rs`
  - [ ] 6.2 Update `crates/nullslop-component/src/lib.rs`:
    - Remove `pub mod picker_highlight;`
    - Remove `use ratatui::style::{Color, Modifier, Style};`
    - Remove the `PICKER_HIGHLIGHT_STYLE` const definition (4 lines)
    - Update crate doc comment if needed (remove mentions of deleted modules)

- [ ] Step 7: Run `cargo test --workspace`
