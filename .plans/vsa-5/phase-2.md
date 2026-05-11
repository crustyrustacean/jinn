# VSA-5 Phase 2: Move Entry Structs to `nullslop-protocol`, Impls + Loaders + Tests to Slices

## Context

Phase 1 deleted 4 empty shell modules, removed the empty `provider_picker` shell from `nullslop-protocol`, and moved `PICKER_HIGHLIGHT_STYLE` + `highlight_text()` to `nullslop-selection-widget`. Phase 2 moves the four picker entry struct definitions (`PickerEntry`, `SessionEntry`, `KeymapEntry`, `StrategyEntry`) into `nullslop-protocol` so that both `nullslop-component` (for `AppState`/`FrontendState`) and slice crates can reference them. Then it moves all `PickerItem` impls, loader functions, render helpers, and tests into their owning slice crates. Finally, it deletes the four picker module directories from `nullslop-component`.

## Scope

### Entry struct definitions → `nullslop-protocol`

Four bare struct definitions move to `nullslop-protocol`. No `PickerItem` impls, no render helpers, no loaders — just the structs and their doc comments. All field types (`String`, `bool`, `u64`, `SessionId`, `PromptStrategyId`, `Intent`, `jiff::Timestamp`) are already available in `nullslop-protocol`.

### `PickerItem` impls + loaders + helpers + tests → domain slices

| Entry Struct | Protocol Module | Slice Destination |
|---|---|---|
| `PickerEntry` | `nullslop-protocol::provider_picker::entries` | `nsslice-provider` |
| `SessionEntry` | `nullslop-protocol::session_picker::entries` | `nsslice-session-management` |
| `KeymapEntry` | `nullslop-protocol::keymap_picker::entries` | `nsslice-picker` |
| `StrategyEntry` | `nullslop-protocol::context_strategy_picker::entries` | `nsslice-picker` |

---

## Step 1: Move `PickerEntry` struct to `nullslop-protocol`

### Step 1.1: Create protocol module

Create `crates/nullslop-protocol/src/provider_picker/mod.rs`:

```rust
//! Provider picker protocol types.

pub mod entries;
```

Create `crates/nullslop-protocol/src/provider_picker/entries.rs` — copy ONLY the bare `PickerEntry` struct definition (10 fields: `provider_id`, `name`, `provider_name`, `backend`, `model`, `is_alias`, `alias_target`, `is_available`, `is_remote`, `is_active`) with its doc comments. No `PickerItem` impl, no functions.

### Step 1.2: Register module in protocol lib.rs

Add to `crates/nullslop-protocol/src/lib.rs`:
- `pub mod provider_picker;` in the module declarations section (between `provider` and `session`)
- `pub use provider_picker::entries::PickerEntry;` in the re-exports section (after `pub use provider::*`)

### Step 1.3: Update `nullslop-component/src/app_state.rs`

Change:
```rust
use crate::provider_picker::entries::PickerEntry;
```
To:
```rust
use nullslop_protocol::PickerEntry;
```

### Step 1.4: Update `nullslop-component/src/provider_picker/mod.rs`

Remove the re-export line `pub use entries::PickerEntry;`. The module still declares `pub mod entries;` and `pub mod loader;`.

### Step 1.5: Update `nullslop-component/src/provider_picker/entries.rs`

Change the `PickerEntry` struct definition to a re-export:
```rust
pub use nullslop_protocol::PickerEntry;
```

Remove the original struct definition. All other code (`PickerItem` impl, `render_provider_row`, `highlight_model_in_label`, `sorted_entries`, `format_footer`, `age_color`, `truncate_line`, `load_provider_entries`, test module) stays.

---

## Step 2: Move `PickerEntry` `PickerItem` impl + helpers + loader + tests to `nsslice-provider`

### Step 2.1: Add dependencies to `nsslice-provider/Cargo.toml`

Add to `[dependencies]`:
```toml
nullslop-protocol = { workspace = true }
nullslop-selection-widget = { workspace = true }
nullslop-services = { workspace = true }
nullslop-providers = { workspace = true }
jiff = { workspace = true }
humantime = { workspace = true }
```

Add to `[dev-dependencies]`:
```toml
nullslop-providers = { workspace = true }  # already has this? check — no, need to add
```

Note: `ratatui` and `unicode-segmentation` are already deps. `rstest` is already a dev-dep.

### Step 2.2: Create `nsslice-provider/src/entries.rs`

Move from `nullslop-component/src/provider_picker/entries.rs`:
- `impl PickerItem for PickerEntry` block
- `render_provider_row()` function
- `highlight_model_in_label()` function
- `sorted_entries()` function
- `format_footer()` function
- `age_color()` function
- `truncate_line()` function
- `load_provider_entries()` function

Update imports:
```rust
use std::ops::Range;

use nullslop_protocol::PickerEntry;
use nullslop_selection_widget::{highlight_text, PickerItem};
use nullslop_services::Services;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
```

Keep the `#[cfg(test)] #[path = "entries_tests.rs"] mod entries_tests;` at the bottom.

### Step 2.3: Create `nsslice-provider/src/entries_tests.rs`

Copy verbatim from `nullslop-component/src/provider_picker/entries_tests.rs`. The test file uses `super::*` so it automatically picks up the new location. Verify the following imports work:
- `nullslop_providers::{ApiKeys, ProviderEntry, ProviderRegistry, ProvidersConfig}` — available via dev-dep
- `super::*` — brings in `PickerEntry` from `nullslop_protocol`

### Step 2.4: Create `nsslice-provider/src/loader.rs`

Move `load_provider_picker_items()` from `nullslop-component/src/provider_picker/loader.rs`. Update imports:

```rust
use nullslop_protocol::PickerEntry;
use nullslop_selection_widget::PickerItem;
use nullslop_services::Services;

use crate::entries::{load_provider_entries, sorted_entries};
```

**Problem:** `load_provider_picker_items` writes to `state.provider.provider_picker.set_items()` which requires `nullslop_component::AppState`. The loader needs `nullslop-component` as a dep (which it already has). So:

```rust
use nullslop_component::AppState;
use nullslop_services::Services;

use crate::entries::{load_provider_entries, sorted_entries};

pub fn load_provider_picker_items(services: &Services, state: &mut AppState) {
    let registry = services.provider_registry.read();
    let api_keys = services.api_keys.read();
    let all = load_provider_entries(&registry, &api_keys, state.provider.model_cache.as_ref());
    let entries = sorted_entries(&all, "", &state.provider.active_provider);
    state.provider.provider_picker.set_items(entries);
}
```

### Step 2.5: Update `nsslice-provider/src/lib.rs`

Add:
```rust
pub mod entries;
pub mod loader;
```

---

## Step 3: Move `SessionEntry` struct to `nullslop-protocol`

### Step 3.1: Create protocol module

Create `crates/nullslop-protocol/src/session_picker/mod.rs`:

```rust
//! Session picker protocol types.

pub mod entries;
```

Create `crates/nullslop-protocol/src/session_picker/entries.rs` — bare `SessionEntry` struct (4 fields: `session_id`, `title`, `updated_at`, `byte_offset`).

### Step 3.2: Register module in protocol lib.rs

- `pub mod session_picker;` in module declarations
- `pub use session_picker::entries::SessionEntry;` in re-exports

### Step 3.3: Update `nullslop-component/src/app_state.rs`

Change:
```rust
use crate::session_picker::entries::SessionEntry;
```
To:
```rust
use nullslop_protocol::SessionEntry;
```

### Step 3.4: Update `nullslop-component/src/session_picker/mod.rs`

Remove `pub use entries::SessionEntry;`.

### Step 3.5: Update `nullslop-component/src/session_picker/entries.rs`

Replace the struct definition with:
```rust
pub use nullslop_protocol::SessionEntry;
```
Keep all other code (`PickerItem` impl, `render_session_row`, `load_session_entries`, `load_session_picker_items`, inline tests).

---

## Step 4: Move `SessionEntry` `PickerItem` impl + loader + tests to `nsslice-session-management`

### Step 4.1: Add dependencies to `nsslice-session-management/Cargo.toml`

Add to `[dependencies]`:
```toml
nullslop-selection-widget = { workspace = true }
nullslop-services = { workspace = true }
jiff = { workspace = true }
```

Note: `nullslop-protocol` and `nullslop-component` are already deps. `rstest` is already a dev-dep.

### Step 4.2: Create `nsslice-session-management/src/entries.rs`

Move from `nullslop-component/src/session_picker/entries.rs`:
- `render_session_row()` function
- `PickerItem` impl for `SessionEntry`
- `load_session_entries()` function
- `load_session_picker_items()` function
- Inline test module

Update imports:
```rust
use std::ops::Range;

use nullslop_component::AppState;
use nullslop_protocol::{SessionEntry, SessionId};
use nullslop_selection_widget::{highlight_text, PickerItem};
use nullslop_services::Services;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
```

### Step 4.3: Update `nsslice-session-management/src/lib.rs`

Add:
```rust
pub mod entries;
```

---

## Step 5: Move `KeymapEntry` struct to `nullslop-protocol`

### Step 5.1: Create protocol module

Create `crates/nullslop-protocol/src/keymap_picker/mod.rs`:

```rust
//! Keymap picker protocol types.

pub mod entries;
```

Create `crates/nullslop-protocol/src/keymap_picker/entries.rs` — bare `KeymapEntry` struct (6 fields: `key_sequence`, `description`, `scope`, `category`, `command: nullslop_protocol::Intent`, `search_text`). Note: `Intent` is already defined in this crate, so just use `crate::Intent` or `Intent` directly.

### Step 5.2: Register module in protocol lib.rs

- `pub mod keymap_picker;` in module declarations
- `pub use keymap_picker::entries::KeymapEntry;` in re-exports

### Step 5.3: Update `nullslop-component/src/app_state.rs`

Change:
```rust
use crate::keymap_picker::entries::KeymapEntry;
```
To:
```rust
use nullslop_protocol::KeymapEntry;
```

### Step 5.4: Update `nullslop-component/src/keymap_picker/mod.rs`

Remove `pub use entries::KeymapEntry;`.

### Step 5.5: Update `nullslop-component/src/keymap_picker/entries.rs`

Replace the struct definition with:
```rust
pub use nullslop_protocol::KeymapEntry;
```
Keep all other code (`PickerItem` impl, `render_keymap_row`, `highlight_text_segment`, inline tests).

---

## Step 6: Move `KeymapEntry` `PickerItem` impl + helpers + tests to `nsslice-picker`

### Step 6.1: Add dependencies to `nsslice-picker/Cargo.toml`

Add to `[dependencies]`:
```toml
nullslop-selection-widget = { workspace = true }
ratatui = { workspace = true }
```

Note: `nullslop-protocol`, `nullslop-component`, `wherror` are already deps. `rstest` and `jiff` are already dev-deps.

### Step 6.2: Create `nsslice-picker/src/keymap_entries.rs`

Move from `nullslop-component/src/keymap_picker/entries.rs`:
- `render_keymap_row()` function
- `highlight_text_segment()` function
- `PickerItem` impl for `KeymapEntry`
- Inline test module (14 tests)

Update imports:
```rust
use std::ops::Range;

use nullslop_protocol::KeymapEntry;
use nullslop_selection_widget::{PickerItem, PICKER_HIGHLIGHT_STYLE};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
```

### Step 6.3: Update `nsslice-picker/src/lib.rs`

Add:
```rust
pub mod keymap_entries;
```

---

## Step 7: Move `StrategyEntry` struct to `nullslop-protocol`

### Step 7.1: Create protocol module

Create `crates/nullslop-protocol/src/context_strategy_picker/mod.rs`:

```rust
//! Context strategy picker protocol types.

pub mod entries;
```

Create `crates/nullslop-protocol/src/context_strategy_picker/entries.rs` — bare `StrategyEntry` struct (4 fields: `strategy_id`, `name`, `description`, `is_active`).

### Step 7.2: Register module in protocol lib.rs

- `pub mod context_strategy_picker;` in module declarations
- `pub use context_strategy_picker::entries::StrategyEntry;` in re-exports

### Step 7.3: Update `nullslop-component/src/app_state.rs`

Change:
```rust
use crate::context_strategy_picker::entries::StrategyEntry;
```
To:
```rust
use nullslop_protocol::StrategyEntry;
```

### Step 7.4: Update `nullslop-component/src/context_strategy_picker/mod.rs`

No re-export of `StrategyEntry` exists (mod.rs only has `pub mod entries;`). No change needed.

### Step 7.5: Update `nullslop-component/src/context_strategy_picker/entries.rs`

Replace the struct definition with:
```rust
pub use nullslop_protocol::StrategyEntry;
```
Keep all other code (`PickerItem` impl, `render_strategy_row`, `load_strategy_entries`, `sorted_strategy_entries`, `load_strategy_picker_items`, `format_strategy_footer`, test module).

---

## Step 8: Move `StrategyEntry` `PickerItem` impl + loader + helpers + tests to `nsslice-picker`

### Step 8.1: Add dependencies to `nsslice-picker/Cargo.toml`

Add to `[dependencies]`:
```toml
nullslop-services = { workspace = true }
```

Note: `nullslop-selection-widget` and `ratatui` are added in Step 6.1. `nullslop-protocol` and `nullslop-component` are already deps.

### Step 8.2: Create `nsslice-picker/src/strategy_entries.rs`

Move from `nullslop-component/src/context_strategy_picker/entries.rs`:
- `render_strategy_row()` function
- `PickerItem` impl for `StrategyEntry`
- `load_strategy_entries()` function
- `sorted_strategy_entries()` function
- `load_strategy_picker_items()` function
- `format_strategy_footer()` function

Update imports:
```rust
use std::ops::Range;

use nullslop_component::AppState;
use nullslop_protocol::{PromptStrategyId, StrategyEntry};
use nullslop_selection_widget::{highlight_text, PickerItem};
use nullslop_services::Services;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
```

### Step 8.3: Create `nsslice-picker/src/strategy_entries_tests.rs`

Copy from `nullslop-component/src/context_strategy_picker/entries_tests.rs`. Update to use `super::*` (which pulls from `strategy_entries.rs`). The test helper `make_entry` constructs `StrategyEntry` directly — it will now resolve via the re-export from `nullslop_protocol`.

Tests that call `load_strategy_entries` use `crate::test_utils::test_services()` in the current location. The slice doesn't have this utility. Solution: the `load_strategy_entries` tests call `nullslop_services::Services::new()` directly instead. The `make_entry` tests don't need services at all.

### Step 8.4: Update `nsslice-picker/src/lib.rs`

Add (in addition to Step 6.3):
```rust
pub mod strategy_entries;
```

---

## Step 9: Update all external consumers

### Step 9.1: `nullslop-tui/src/render.rs`

Change:
```rust
use nullslop_component::provider_picker::entries;
```
To:
```rust
use nsslice_provider::entries;
```

Change:
```rust
use nullslop_component::context_strategy_picker::entries;
```
To:
```rust
use nsslice_picker::strategy_entries as entries;
```

Note: This requires `nullslop-tui` to depend on `nsslice-provider` and `nsslice-picker`. Check if these deps exist — if not, add to `nullslop-tui/Cargo.toml`.

### Step 9.2: `nullslop-tui/src/render_tests.rs`

Change all `nullslop_component::provider_picker::load_provider_picker_items(...)` to `nsslice_provider::loader::load_provider_picker_items(...)`.

Change all `nullslop_component::context_strategy_picker::entries::load_strategy_picker_items(...)` to `nsslice_picker::strategy_entries::load_strategy_picker_items(...)`.

Change `use nullslop_component::keymap_picker::KeymapEntry` to `use nullslop_protocol::KeymapEntry`.

### Step 9.3: `nullslop-tui/src/keymap.rs`

Change all `nullslop_component::keymap_picker::KeymapEntry` references to `nullslop_protocol::KeymapEntry`.

### Step 9.4: `nullslop-intent/src/handler_tests.rs`

Change:
```rust
use nullslop_component::keymap_picker::entries::KeymapEntry;
```
To:
```rust
use nullslop_protocol::KeymapEntry;
```

### Step 9.5: `nsslice-picker/src/intent.rs` (test section)

Change:
```rust
use nullslop_component::context_strategy_picker::entries::StrategyEntry;
use nullslop_component::keymap_picker::entries::KeymapEntry;
use nullslop_component::provider_picker::entries::PickerEntry;
use nullslop_component::session_picker::entries::SessionEntry;
```
To:
```rust
use nullslop_protocol::{KeymapEntry, PickerEntry, SessionEntry, StrategyEntry};
```

### Step 9.6: `nsslice-picker/src/validator.rs` (test section)

Change all four `use nullslop_component::..._picker::entries::*Entry` to `use nullslop_protocol::*Entry`.

### Step 9.7: `nullslop-tui/Cargo.toml`

`nsslice-provider` is already a dep. Add `nsslice-picker`:
```toml
nsslice-picker = { workspace = true }
```

### Step 9.8: `nullslop-provider-actor/src/lib.rs`

Change:
```rust
use nullslop_component::provider_picker::loader::load_provider_picker_items;
```
To:
```rust
use nsslice_provider::loader::load_provider_picker_items;
```

Add to `nullslop-provider-actor/Cargo.toml`:
```toml
nsslice-provider = { workspace = true }
```

### Step 9.9: Workspace `Cargo.toml`

All slice workspace entries confirmed present.

---

## Step 10: Delete picker modules from `nullslop-component`

### Step 10.1: Delete four directories

```bash
rm -rf crates/nullslop-component/src/provider_picker
rm -rf crates/nullslop-component/src/session_picker
rm -rf crates/nullslop-component/src/keymap_picker
rm -rf crates/nullslop-component/src/context_strategy_picker
```

### Step 10.2: Update `nullslop-component/src/lib.rs`

Remove:
```rust
pub mod context_strategy_picker;
pub mod keymap_picker;
pub mod provider_picker;
pub mod session_picker;
```

### Step 10.3: Verify no dangling references

```bash
grep -rn 'crate::provider_picker\|crate::session_picker\|crate::keymap_picker\|crate::context_strategy_picker' crates/nullslop-component/src/
```

Must return empty.

---

## Step 11: Run `cargo test --workspace`

Verify all tests pass.

---

## Acceptance Criteria

1. `nullslop-component/src/{provider_picker,session_picker,keymap_picker,context_strategy_picker}/` directories do not exist
2. `PickerEntry` is defined in `nullslop-protocol/src/provider_picker/entries.rs`
3. `SessionEntry` is defined in `nullslop-protocol/src/session_picker/entries.rs`
4. `KeymapEntry` is defined in `nullslop-protocol/src/keymap_picker/entries.rs`
5. `StrategyEntry` is defined in `nullslop-protocol/src/context_strategy_picker/entries.rs`
6. All four are re-exported from `nullslop-protocol` crate root
7. `nsslice-provider` contains: `PickerItem` impl for `PickerEntry`, `render_provider_row`, `highlight_model_in_label`, `sorted_entries`, `format_footer`, `age_color`, `truncate_line`, `load_provider_entries`, `load_provider_picker_items`, and all tests
8. `nsslice-session-management` contains: `PickerItem` impl for `SessionEntry`, `render_session_row`, `load_session_entries`, `load_session_picker_items`, and inline tests
9. `nsslice-picker` contains `keymap_entries` module: `PickerItem` impl for `KeymapEntry`, `render_keymap_row`, `highlight_text_segment`, and 14 inline tests
10. `nsslice-picker` contains `strategy_entries` module: `PickerItem` impl for `StrategyEntry`, `render_strategy_row`, `load_strategy_entries`, `sorted_strategy_entries`, `load_strategy_picker_items`, `format_strategy_footer`, and all tests
11. `nullslop-component/src/app_state.rs` imports all four entry types from `nullslop_protocol::`
12. No remaining `use crate::provider_picker`, `use crate::session_picker`, `use crate::keymap_picker`, or `use crate::context_strategy_picker` in `nullslop-component`
13. `nullslop-tui/src/render.rs` imports `entries` from `nsslice_provider` and `nsslice_picker`
14. `nullslop-tui/src/keymap.rs` uses `nullslop_protocol::KeymapEntry`
15. All slice test files (`nsslice-picker/intent.rs`, `nsslice-picker/validator.rs`, `nullslop-intent/handler_tests.rs`) import entry types from `nullslop_protocol`
16. `nullslop-provider-actor` imports `load_provider_picker_items` from `nsslice_provider::loader`
17. `cargo test --workspace` passes — no regressions

---

## Implementation Steps (checklist)

**Note:** `PickerItem` impls could not move to slice crates due to Rust's orphan rule (foreign trait on foreign type). They live in `nullslop-protocol` alongside the struct definitions. Loaders, sorting, formatting, and tests moved to slices as planned.

- [x] Step 1: Move `PickerEntry` struct to `nullslop-protocol`
  - [x] 1.1 Create `crates/nullslop-protocol/src/provider_picker/mod.rs` and `entries.rs` (struct + `PickerItem` impl + render helpers)
  - [x] 1.2 Add `pub mod provider_picker;` + re-export to protocol `lib.rs`
  - [x] 1.3 Update `app_state.rs` import
  - [x] 1.4–1.5 Deleted from component (done atomically with Step 10)

- [x] Step 2: Move `PickerEntry` loader + sorting + formatting + tests to `nsslice-provider`
  - [x] 2.1 Add deps to `nsslice-provider/Cargo.toml` (including `nullslop-providers` as regular dep per review fix)
  - [x] 2.2 Create `nsslice-provider/src/entries.rs` (sorting, formatting, loader fn)
  - [x] 2.3 Create `nsslice-provider/src/entries_tests.rs`
  - [x] 2.4 Create `nsslice-provider/src/loader.rs`
  - [x] 2.5 Update `nsslice-provider/src/lib.rs`

- [x] Step 3: Move `SessionEntry` struct to `nullslop-protocol`
  - [x] 3.1 Create `crates/nullslop-protocol/src/session_picker/mod.rs` and `entries.rs` (struct + `PickerItem` impl + render helper)
  - [x] 3.2 Add `pub mod session_picker;` + re-export to protocol `lib.rs`
  - [x] 3.3 Update `app_state.rs` import
  - [x] 3.4–3.5 Deleted from component (done atomically with Step 10)

- [x] Step 4: Move `SessionEntry` loader + tests to `nsslice-session-management`
  - [x] 4.1 Add deps to `nsslice-session-management/Cargo.toml` (including `ratatui` per review fix)
  - [x] 4.2 Create `nsslice-session-management/src/entries.rs` (loader + inline tests)
  - [x] 4.3 Update `nsslice-session-management/src/lib.rs`

- [x] Step 5: Move `KeymapEntry` struct to `nullslop-protocol`
  - [x] 5.1 Create `crates/nullslop-protocol/src/keymap_picker/mod.rs` and `entries.rs` (struct + `PickerItem` impl + render helpers)
  - [x] 5.2 Add `pub mod keymap_picker;` + re-export to protocol `lib.rs`
  - [x] 5.3 Update `app_state.rs` import
  - [x] 5.4–5.5 Deleted from component (done atomically with Step 10)

- [x] Step 6: Move `KeymapEntry` tests to `nsslice-picker`
  - [x] 6.1 Add deps to `nsslice-picker/Cargo.toml`
  - [x] 6.2 Create `nsslice-picker/src/keymap_entries.rs` (14 tests)
  - [x] 6.3 Update `nsslice-picker/src/lib.rs`

- [x] Step 7: Move `StrategyEntry` struct to `nullslop-protocol`
  - [x] 7.1 Create `crates/nullslop-protocol/src/context_strategy_picker/mod.rs` and `entries.rs` (struct + `PickerItem` impl + render helper)
  - [x] 7.2 Add `pub mod context_strategy_picker;` + re-export to protocol `lib.rs`
  - [x] 7.3 Update `app_state.rs` import

- [x] Step 8: Move `StrategyEntry` loader + sorting + formatting + tests to `nsslice-picker`
  - [x] 8.1 Add `nullslop-services` dep to `nsslice-picker/Cargo.toml`
  - [x] 8.2 Create `nsslice-picker/src/strategy_entries.rs`
  - [x] 8.3 Create `nsslice-picker/src/strategy_entries_tests.rs` (using `nullslop_services::Services::new()` per review fix)
  - [x] 8.4 Update `nsslice-picker/src/lib.rs`

- [x] Step 9: Update all external consumers
  - [x] 9.1 Update `nullslop-tui/src/render.rs` imports
  - [x] 9.2 Update `nullslop-tui/src/render_tests.rs` imports
  - [x] 9.3 Update `nullslop-tui/src/keymap.rs` imports
  - [x] 9.4 Update `nullslop-intent/src/handler_tests.rs` imports
  - [x] 9.5 Update `nsslice-picker/src/intent.rs` test imports
  - [x] 9.6 Update `nsslice-picker/src/validator.rs` test imports
  - [x] 9.7 Add `nsslice-picker` dep to `nullslop-tui/Cargo.toml`
  - [x] 9.8 Update `nullslop-provider-actor/src/lib.rs` to use `nsslice_provider::loader`
  - [x] 9.9 Add `nsslice-provider` dep to `nullslop-provider-actor/Cargo.toml`

- [x] Step 10: Delete picker modules from `nullslop-component`
  - [x] 10.1 Delete 4 directories
  - [x] 10.2 Update `nullslop-component/src/lib.rs`
  - [x] 10.3 Verify no dangling references

- [x] Step 11: Run `cargo test --workspace` — 1,391 tests pass, 0 failures
