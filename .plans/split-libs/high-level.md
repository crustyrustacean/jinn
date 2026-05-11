# split-libs: Split long files into focused submodules

## Problem

Several files in the codebase are excessively long and define many unrelated types, violating single-responsibility and making the code harder to navigate. The worst offenders are `lib.rs` files that cram an entire crate's domain into a single file, and actor/state files that mix multiple concerns (handlers, persistence, session management, UI logic) in one place.

This plan focuses on the **8 highest-impact files** (3 `lib.rs` files + 2 `actor.rs` files + 3 other large files). Lower-priority files that are long but cohesive (e.g., `split_borders.rs`, `in_memory.rs`, `picker/intent.rs`, `chat-log/element.rs`) are excluded — they define a single algorithm or a family of related functions where splitting gives marginal benefit.

## Approach

- Use **multiple `impl` blocks across files** within the same crate. Handler methods stay as methods on the actor/struct but live in separate files. This is idiomatic Rust and avoids changing the public API.
- **One commit per file split.** When done splitting one file into many submodules, commit before moving to the next.
- **Tests stay where they are** (inline `mod tests`) unless the test module itself is very large. The goal is splitting production code, not reorganizing tests.
- **Preserve all existing `pub` visibility.** This is purely an internal reorganization — no public API changes.

## Phases

- [x] Phase 1: Split `nsslice-tools/src/lib.rs` (1496 → ~350 + builtin.rs)
  - [x] Extract `builtin.rs` — `BuiltinToolEntry` type alias, `builtin_tools()` function, and all 4 builtin tool definition+execute pairs (`echo`, `get_time`, `file_read`, `file_write`) into `src/builtin.rs`
  - [x] Update `lib.rs` — add `mod builtin;` and `use builtin::BuiltinToolEntry;`, keep actor types + `spawn()` in root
  - [x] Commit

- [x] Phase 2: Split `nsslice-llm/src/lib.rs` (1289 → ~250 + session.rs)
  - [x] Extract `session.rs` — `SessionState` enum, `SessionData` struct + impl into `src/session.rs`
  - [x] Update `lib.rs` — add `mod session;` and re-export `SessionState`, `SessionData`
  - [x] Commit

- [x] Phase 3: Split `nsslice-chat-input-box-protocol/src/lib.rs` (587 → ~250 + autocomplete.rs)
  - [x] Extract `autocomplete.rs` — `AutocompleteMatch` struct, `AutocompleteState` struct + impl, and all autocomplete-related `impl ChatInputBoxState` methods (autocomplete accessors, navigation, completion, expansion, filtering, token helpers)
  - [x] Update `lib.rs` — add `mod autocomplete;`, re-export `AutocompleteMatch` and `AutocompleteState`, keep core buffer logic (constructors, cursor movement, text mutation, navigation helpers)
  - [x] Commit

- [x] Phase 4: Split `nsslice-session-management/src/actor.rs` (1494 → ~150 + handler submodules)
  - [x] Extract `src/actor/handlers/command.rs` — `EnqueueAction` enum + all command handler methods as `impl SessionPersistenceActor`
  - [x] Extract `src/actor/handlers/event.rs` — all event handler methods as `impl SessionPersistenceActor` (stream token/completed, tool use/call/streaming/execution)
  - [x] Extract `src/actor/handlers/persistence.rs` — `on_save_requested()`, `on_load_requested()` as `impl SessionPersistenceActor`
  - [x] Create `src/actor/handlers/mod.rs` — re-exports
  - [x] Restructure: rename `actor.rs` → `src/actor/mod.rs`, add `mod handlers;`, keep `Actor` impl + dispatchers only
  - [x] Commit

- [x] Phase 5: Split `nsslice-context/src/actor.rs` (1380 → ~80 + handler submodules)
  - [x] Extract `src/actor/handlers/assembly.rs` — `ensure_strategy()`, `on_assemble_prompt()` as `impl PromptAssemblyActor`
  - [x] Extract `src/actor/handlers/strategy.rs` — `on_prompt_strategy_switched()`, `handle_switch_prompt_strategy()`, `handle_restore_strategy_state()`
  - [x] Extract `src/actor/handlers/pinning.rs` — `handle_pin_chat_entry()`, `handle_unpin_chat_entry()`
  - [x] Extract `src/actor/handlers/caching.rs` — `on_tools_registered()`, `on_prompt_templates_loaded()`
  - [x] Create `src/actor/handlers/mod.rs` — re-exports
  - [x] Restructure: rename `actor.rs` → `src/actor/mod.rs`, add `mod handlers;`, keep `Actor` impl + dispatchers only
  - [x] Commit

- [x] Phase 6: Split `nullslop-tui/src/app.rs` (971 → ~250 + extracted modules)
  - [x] Extract `src/app/pinned_pane.rs` — `PaneFocus` enum, `CHAT_PANE` const, `open_pinned_pane()`, `close_pinned_pane()`, `toggle_pinned_pane()` as methods on `TuiApp`
  - [x] Extract `src/app/builder.rs` — `TuiAppBuilder` struct + impl
  - [x] Extract `src/app/signals.rs` — `TuiSignalsSnapshot` struct + `from_state()` impl
  - [x] Restructure: rename `app.rs` → `src/app/mod.rs`, keep `TuiApp` struct + `handle_msg()` + `route_intent()` + `render()` + `scope_for_mode()`
  - [x] Commit

- [x] Phase 7: Split `nullslop-component/src/chat_session/state.rs` (713 → ~200 + impl submodules)
  - [x] Extract `src/chat_session/state/streaming.rs` — streaming-related `impl ChatSessionState` methods (`begin_streaming`, `append_stream_token`, `finish_streaming`, `cancel_streaming`, `cancel_stream_and_drain`, `is_streaming`, tool call methods)
  - [x] Extract `src/chat_session/state/scroll.rs` — scrolling-related `impl ChatSessionState` methods (`scroll_offset`, `is_at_bottom`, `scroll_up`, `scroll_down`, `reset_scroll`, `scroll_to_top`, `scroll_to_bottom`, `set_last_max_offset`)
  - [x] Extract `src/chat_session/state/selection.rs` — selection-related `impl ChatSessionState` methods (`select_next_entry`, `select_prev_entry`, `clear_selection`, `selected_entry_index`, `selected_entry`, `selected_entry_id`)
  - [x] Extract `src/chat_session/state/queue.rs` — queue-related `impl ChatSessionState` methods (`queue`, `queue_len`, `enqueue_message`, `dequeue_message`, `drain_queue`)
  - [x] Restructure: rename `state.rs` → `src/chat_session/state/mod.rs`, keep struct definitions + core methods (`new`, `push_entry`, `chat_input`, `history`, `pin_entry`, `unpin_entry`, `pinned_entries`, `restore_history`, strategy methods, `is_idle`, assembling/sending methods)
  - [x] Commit

- [x] Phase 8: Split `nsslice-session-management-protocol/src/session_store.rs` (792 → ~200 + extracted modules)
  - [x] Extract `src/session_store/jsonl.rs` — `JsonlSessionStore` struct + `Default` impl + `SessionStore` impl (the concrete file-based implementation)
  - [x] Extract `src/session_store/service.rs` — `SessionStoreService` struct + impl (the `Arc<dyn Trait>` service wrapper)
  - [x] Restructure: rename `session_store.rs` → `src/session_store/mod.rs`, keep `SessionStoreError`, `SessionStore` trait, and `Debug` impl for `dyn SessionStore`
  - [x] Commit

## Acceptance Criteria

- All 8 files have been split into focused submodules
- Each original file reduced to ~200–350 lines (struct/enum definitions + dispatchers + re-exports)
- No public API changes — all `pub` items remain `pub` and accessible from the same paths
- `just test` passes after every commit
- `just lint` passes after every commit
