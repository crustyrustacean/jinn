# VSA-5 Phase 3: Move Render Functions + `expand_tokens` to Slices

## Context

Phase 2 moved entry struct definitions + `PickerItem` impls to `nullslop-protocol`, and loaders/sorting/formatting/tests to domain slice crates. Phase 3 moves the remaining domain-specific code out of `nullslop-component` and `nullslop-tui`:

1. `expand_tokens()` function + its 13 tests from `nullslop-component/src/prompt_template/mod.rs` → `nullslop-prompt-template`
2. Picker render functions from `nullslop-tui/src/render.rs` → domain slices
3. Autocomplete popup render from `nullslop-tui/src/render.rs` → `nsslice-chat-input-box`
4. Corresponding render tests from `nullslop-tui/src/render_tests.rs` → domain slices

After Phase 3, `nullslop-component/src/prompt_template/` is re-exports only, and `nullslop-tui/src/render.rs` dispatches to slice render functions.

---

## Step 1: Move `expand_tokens()` to `nullslop-prompt-template`

### Step 1.1: Add `unicode-segmentation` dep to `nullslop-prompt-template`

Add to `[dependencies]` in `crates/nullslop-prompt-template/Cargo.toml`:
```toml
unicode-segmentation = { workspace = true }
```

### Step 1.2: Create `crates/nullslop-prompt-template/src/expand.rs`

Move `expand_tokens()` from `nullslop-component/src/prompt_template/mod.rs` to this file. Update imports:
```rust
use nullslop_protocol::PromptTemplate;
use crate::PromptTemplateStore;
```

The function body is unchanged. It's a `pub fn` so it's part of the crate's public API.

### Step 1.3: Move the 13 tests to `crates/nullslop-prompt-template/src/expand.rs`

Move the `#[cfg(test)] mod expand_tokens_tests` block into the new file. Update:
- `use super::*;` → picks up `expand_tokens` and `PromptTemplateStore` from the same module
- `use nullslop_protocol::PromptTemplate;` for constructing test templates

### Step 1.4: Update `crates/nullslop-prompt-template/src/lib.rs`

Add `pub mod expand;` to module declarations.
Add `pub use expand::expand_tokens;` to re-exports.

### Step 1.5: Update `nullslop-component/src/prompt_template/mod.rs`

Replace the `expand_tokens` function definition + test module with a re-export:
```rust
pub use nullslop_prompt_template::expand_tokens;
```

Keep all existing re-exports (`PromptTemplateStore`, `PromptTemplateParseError`, etc.).

### Step 1.6: Verify no external consumer breakage

`expand_tokens` is currently not called outside `nullslop-component`. The re-export preserves the path `nullslop_component::prompt_template::expand_tokens` for any future use. `nsslice-chat-input-box` imports `PromptTemplateStore` via the existing re-export chain — no change needed.

---

## Step 2: Move picker render functions to slices

### Step 2.1: Create `nsslice-provider/src/render.rs`

Move `render_provider_picker()` from `nullslop-tui/src/render.rs`. Update imports:
```rust
use nullslop_component::AppState;
use nullslop_selection_widget::SelectionWidget;
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::entries;
```

Make it `pub fn`. The function reads `state.provider.provider_picker`, `state.provider.last_refreshed_at`, and calls `entries::format_footer`.

### Step 2.2: Update `nsslice-provider/src/lib.rs`

Add `pub mod render;`.

### Step 2.3: Create `nsslice-session-management/src/render.rs`

Move `render_session_picker()` from `nullslop-tui/src/render.rs`. Update imports:
```rust
use nullslop_component::AppState;
use nullslop_selection_widget::SelectionWidget;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::Line;
```

Make it `pub fn`.

### Step 2.4: Update `nsslice-session-management/src/lib.rs`

Add `pub mod render;`.

### Step 2.5: Create `nsslice-picker/src/render.rs`

Move both `render_keymap_picker()` and `render_context_strategy_picker()` from `nullslop-tui/src/render.rs`. Update imports:
```rust
use nullslop_component::AppState;
use nullslop_selection_widget::SelectionWidget;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::Line;

use crate::strategy_entries;
```

Make both `pub fn`.

### Step 2.6: Update `nsslice-picker/src/lib.rs`

Add `pub mod render;`.

### Step 2.7: Add `nsslice-session-management` dep to `nullslop-tui/Cargo.toml`

`nullslop-tui` already depends on `nsslice-provider` and `nsslice-picker`. Add:
```toml
nsslice-session-management = { workspace = true }
```

---

## Step 3: Move autocomplete popup render to `nsslice-chat-input-box`

### Step 3.1: Add deps to `nsslice-chat-input-box/Cargo.toml`

Add to `[dependencies]`:
```toml
nullslop-selection-widget = { workspace = true }
```

Note: `ratatui` is already a dep. `nullslop-component` is already a dep (for `AppState`).

### Step 3.2: Create `nsslice-chat-input-box/src/autocomplete_render.rs`

Move from `nullslop-tui/src/render.rs`:
- `render_autocomplete_popup()` function
- `scroll_window()` function
- All autocomplete constants: `AUTOCOMPLETE_MAX_VISIBLE`, `AUTOCOMPLETE_MIN_WIDTH`, `AUTOCOMPLETE_MAX_WIDTH_FRAC`, `AUTOCOMPLETE_NAME_DESC_SEP`, `AUTOCOMPLETE_NO_MATCHES`

Update imports:
```rust
use nullslop_component::AppState;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph};
```

Make `render_autocomplete_popup` and `scroll_window` `pub`. Make constants `pub` as needed.

### Step 3.3: Update `nsslice-chat-input-box/src/lib.rs`

Add `pub mod autocomplete_render;`.

---

## Step 4: Update `nullslop-tui/src/render.rs`

### Step 4.1: Replace render functions with dispatch calls

Replace `render_provider_picker` body with delegation to slice:
```rust
fn render_provider_picker(frame: &mut Frame<'_>, area: Rect, state: &nullslop_component::AppState) {
    nsslice_provider::render::render_provider_picker(frame, area, state);
}
```

Same pattern for:
- `render_session_picker` → `nsslice_session_management::render::render_session_picker`
- `render_keymap_picker` → `nsslice_picker::render::render_keymap_picker`
- `render_context_strategy_picker` → `nsslice_picker::render::render_context_strategy_picker`

### Step 4.2: Replace autocomplete render call

In the main `render()` function, change:
```rust
render_autocomplete_popup(frame, layout.input, &state);
```
To:
```rust
nsslice_chat_input_box::autocomplete_render::render_autocomplete_popup(frame, layout.input, &state);
```

### Step 4.3: Remove moved functions and constants

Delete from `render.rs`:
- `render_provider_picker` function body (keep thin wrapper or inline the call)
- `render_context_strategy_picker` function body
- `render_keymap_picker` function body
- `render_session_picker` function body
- `render_autocomplete_popup` function
- `scroll_window` function
- All autocomplete constants: `AUTOCOMPLETE_MAX_VISIBLE`, `AUTOCOMPLETE_MIN_WIDTH`, `AUTOCOMPLETE_MAX_WIDTH_FRAC`, `AUTOCOMPLETE_NAME_DESC_SEP`, `AUTOCOMPLETE_NO_MATCHES`

Two approaches:
- **Option A (thin wrappers):** Keep the private `fn render_*_picker` as one-line delegation functions. `render_picker()` already calls them, so no structural change there.
- **Option B (inline calls):** Remove the wrapper functions entirely and call slice functions directly from `render_picker()` and `render()`.

Recommendation: **Option A** — keeps `render_picker()` clean and avoids importing 4 slice render paths in one match arm.

---

## Step 5: Move render tests to slices

### Step 5.1: Move provider picker render tests to `nsslice-provider`

Create `crates/slices/nsslice-provider/src/render_tests.rs` (or inline in `render.rs`).

Move these tests + helpers from `render_tests.rs`:
- `picker_state_with_ollama()` helper
- `load_picker_items()` helper
- `render_provider_picker_shows_telescope_layout`
- `larger_terminal_gets_taller_popup` (note: tests `compute_popup_rect`, not provider-specific — consider keeping in `nullslop-tui`)
- `small_terminal_uses_75_percent_height` (same — tests `compute_popup_rect`)
- `render_provider_picker_uses_dark_gray_border`
- `render_provider_picker_shows_active_model_marker`

Update imports:
```rust
use nullslop_component::AppState;
use nullslop_services::Services;
use nullslop_providers::{ProviderEntry, ProvidersConfig};
use nsslice_provider::loader::load_provider_picker_items;
use nullslop_selection_widget::compute_popup_rect;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::style::Color;
```

For the test that calls `render_provider_picker`: it now calls `crate::render::render_provider_picker`.

### Step 5.2: Move context strategy picker render tests to `nsslice-picker`

Create `crates/slices/nsslice-picker/src/render_tests.rs` (or `strategy_render_tests.rs`).

Move:
- `strategy_picker_state()` helper
- `render_context_strategy_picker_shows_telescope_layout`
- `render_context_strategy_picker_shows_active_marker`
- `render_context_strategy_picker_shows_footer_with_current_strategy`

### Step 5.3: Move keymap picker render tests to `nsslice-picker`

Same file as Step 5.2 (or separate `keymap_render_tests.rs`).

Move:
- `keymap_picker_state()` helper
- `render_keymap_picker_shows_telescope_layout`
- `render_keymap_picker_footer_shows_current_scope`
- `render_keymap_picker_footer_shows_all_scopes`

### Step 5.4: Move autocomplete render tests to `nsslice-chat-input-box`

Create `crates/slices/nsslice-chat-input-box/src/autocomplete_render_tests.rs`.

Move:
- `state_with_autocomplete()` helper
- `buffer_line()` helper
- `render_autocomplete_popup_shows_matches`
- `render_autocomplete_popup_highlights_selected`
- `render_autocomplete_popup_shows_no_matches_message`
- `render_autocomplete_popup_positioned_above_input`
- `render_autocomplete_popup_anchored_at_dollar`
- `render_autocomplete_popup_width_based_on_content`
- `render_autocomplete_popup_does_not_render_when_inactive`

### Step 5.5: Tests that stay in `nullslop-tui/src/render_tests.rs`

These tests exercise cross-cutting concerns, not domain-specific rendering:
- `app_layout_meets_min_size`, `app_layout_too_small`, `init_tab_manager_has_two_tabs`, `app_layout_includes_indicator_row`, `app_layout_queue_area_*`, `app_layout_includes_status_bar` — layout tests
- `cell_inside_selection_is_inverted`, `cell_outside_selection_is_unchanged`, `cell_inside_clamped_selection_is_inverted`, `cell_at_raw_anchor_not_inverted`, `selection_highlight_does_nothing_when_idle`, `reset_bg_cell_gets_explicit_colors`, `distinct_color_cell_gets_swapped` — selection highlight tests
- `clipboard_*` — clipboard tests
- `render_registers_content_rect_for_selectable_chat_log`, `picker_popup_rect_is_selectable`, `content_area_rect_is_selectable` — element-driven selectable rect tests
- `larger_terminal_gets_taller_popup`, `small_terminal_uses_75_percent_height` — popup sizing tests (exercise `compute_popup_rect` from `nullslop-selection-widget`, not domain-specific)
- `frame_area()` helper stays

Update remaining tests that call moved render functions directly (e.g., `render_provider_picker` is still a thin wrapper in `render.rs`, so calls via `super::*` still work).

---

## Step 6: Run `cargo test --workspace`

Verify all tests pass.

---

## Acceptance Criteria

1. `expand_tokens()` is defined in `nullslop-prompt-template/src/expand.rs` with all 13 tests
2. `nullslop-component/src/prompt_template/mod.rs` is re-exports only (no function bodies, no tests)
3. `nullslop-component/src/prompt_template/mod.rs` re-exports `expand_tokens` from `nullslop_prompt_template`
4. `nsslice-provider/src/render.rs` contains `render_provider_picker()`
5. `nsslice-session-management/src/render.rs` contains `render_session_picker()`
6. `nsslice-picker/src/render.rs` contains `render_keymap_picker()` and `render_context_strategy_picker()`
7. `nsslice-chat-input-box/src/autocomplete_render.rs` contains `render_autocomplete_popup()`, `scroll_window()`, and autocomplete constants
8. `nullslop-tui/src/render.rs` dispatches to slice render functions (thin wrappers or inline calls)
9. `nullslop-tui/src/render.rs` no longer contains `render_autocomplete_popup`, `scroll_window`, or autocomplete constants
10. Provider picker render tests (4 tests + 2 helpers) live in `nsslice-provider`
11. Context strategy picker render tests (3 tests + 1 helper) live in `nsslice-picker`
12. Keymap picker render tests (3 tests + 1 helper) live in `nsslice-picker`
13. Autocomplete render tests (7 tests + 2 helpers) live in `nsslice-chat-input-box`
14. Layout, selection highlight, clipboard, and selectable rect tests remain in `nullslop-tui/src/render_tests.rs`
15. `nullslop-tui/Cargo.toml` depends on `nsslice-session-management`
16. `nsslice-chat-input-box/Cargo.toml` depends on `nullslop-selection-widget`
17. `nullslop-prompt-template/Cargo.toml` depends on `unicode-segmentation`
18. `cargo test --workspace` passes — no regressions

---

## Implementation Steps (checklist)

- [ ] Step 1: Move `expand_tokens()` to `nullslop-prompt-template`
  - [ ] 1.1 Add `unicode-segmentation` dep
  - [ ] 1.2 Create `expand.rs` with function + tests
  - [ ] 1.3 Update `lib.rs` — add `pub mod expand;` + re-export
  - [ ] 1.4 Update `nullslop-component/src/prompt_template/mod.rs` — replace with re-export
  - [ ] 1.5 Verify no external consumer breakage

- [ ] Step 2: Move picker render functions to slices
  - [ ] 2.1 Create `nsslice-provider/src/render.rs` with `render_provider_picker`
  - [ ] 2.2 Update `nsslice-provider/src/lib.rs` — add `pub mod render;`
  - [ ] 2.3 Create `nsslice-session-management/src/render.rs` with `render_session_picker`
  - [ ] 2.4 Update `nsslice-session-management/src/lib.rs` — add `pub mod render;`
  - [ ] 2.5 Create `nsslice-picker/src/render.rs` with both picker render functions
  - [ ] 2.6 Update `nsslice-picker/src/lib.rs` — add `pub mod render;`
  - [ ] 2.7 Add `nsslice-session-management` dep to `nullslop-tui/Cargo.toml`

- [ ] Step 3: Move autocomplete popup render to `nsslice-chat-input-box`
  - [ ] 3.1 Add `nullslop-selection-widget` dep to `nsslice-chat-input-box/Cargo.toml`
  - [ ] 3.2 Create `autocomplete_render.rs` with function + constants
  - [ ] 3.3 Update `lib.rs` — add `pub mod autocomplete_render;`

- [ ] Step 4: Update `nullslop-tui/src/render.rs`
  - [ ] 4.1 Replace render function bodies with dispatch calls
  - [ ] 4.2 Replace autocomplete render call with slice call
  - [ ] 4.3 Remove moved functions and constants
  - [ ] 4.4 Remove unused imports

- [ ] Step 5: Move render tests to slices
  - [ ] 5.1 Move provider picker render tests to `nsslice-provider`
  - [ ] 5.2 Move context strategy picker render tests to `nsslice-picker`
  - [ ] 5.3 Move keymap picker render tests to `nsslice-picker`
  - [ ] 5.4 Move autocomplete render tests to `nsslice-chat-input-box`
  - [ ] 5.5 Verify remaining tests in `render_tests.rs` still compile

- [ ] Step 6: Run `cargo test --workspace`
