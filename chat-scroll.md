# Chat Scroll Feature — Comprehensive Behavior Spec

## Core Data Model

- **`scroll_offset: Option<u16>`** — `None` means auto-scroll (bottom); `Some(n)` means scrolled up to line offset `n`
- **`selected_entry_index: Option<usize>`** — `None` only when history is empty; always `Some(n)` when entries exist
- **`entry_line_ranges: Vec<(u16, u16)>`** — per-entry wrapped line ranges, stored by renderer each frame
- **`viewport_height: u16`**, **`blank_count: u16`**, **`last_max_offset: u16`** — renderer feedback for scroll math

## Key Invariants

1. **Always-selected**: When history is non-empty, exactly one entry is always selected
2. **Smart auto-scroll**: `push_entry` only resets scroll + advances cursor if cursor was on the previous last entry
3. **Renderer stores viewport state** each frame so intent handlers can compute visibility

## Bindings & Expected Behavior

| Binding | Intent | Expected Behavior |
|---------|--------|-------------------|
| `j` / `↓` | `ChatEntrySelectNext` | Move cursor to next entry. If at last visible entry in viewport, page down (scroll by viewport_height), then advance cursor by **exactly 1** (not jump to first visible in new viewport). Clamps at last entry in history — no wrapping. |
| `k` / `↑` | `ChatEntrySelectPrev` | Move cursor to prev entry. If at first visible entry in viewport, page up (scroll by viewport_height), then move cursor back by **exactly 1**. Clamps at entry 0 — no wrapping. |
| `ctrl-d` | `ScrollDown` | Scroll viewport down by viewport_height. Move cursor to last visible entry in new viewport. |
| `ctrl-u` | `ScrollUp` | Scroll viewport up by viewport_height. Move cursor to first visible entry in new viewport. |
| `G` | `ScrollToBottom` | Scroll viewport to very bottom (scroll_offset = None). Move cursor to **last entry** in history. This ensures `was_at_last = true` for subsequent `push_entry` calls, re-enabling auto-scroll. |
| `gg` | `ScrollToTop` | Scroll viewport to very top (scroll_offset = 0). Move cursor to **first entry** in history. |
| Mouse scroll down | `MouseScrollDown` | Scroll down by 3 lines. Move cursor to last visible entry in new viewport. |
| Mouse scroll up | `MouseScrollUp` | Scroll up by 3 lines. Move cursor to first visible entry in new viewport. |
| Sidebar pin nav | `SidebarMoveDown/Up` | When a pinned entry is selected in sidebar, also sync the chat log cursor to that pinned entry and flag renderer to scroll-to-selected. |
| `esc` (Normal mode) | `NormalEscape` | No-op (selection is always active, no longer clears it). |

## Auto-Scroll Behavior

**Expected**:
1. Empty session → cursor is `None`
2. First entry arrives → cursor becomes `Some(0)`, scroll resets to bottom
3. Each subsequent entry → check `was_at_last`: if cursor was on `history.len() - 1`, advance cursor to the new entry and reset scroll. Otherwise, append silently.
4. This applies to ALL entry sources: `push_entry` (user, echo actor, tool results), `begin_streaming` (assistant entry), `begin_thinking` (thinking entry inserted before assistant)

**Production flow for one turn**:
1. User submits message → `EnqueueUserMessage` → session actor calls `push_entry(user)` → cursor advances
2. Session actor calls `begin_sending()`
3. Echo actor (subscribed to `ChatEntrySubmitted`) calls `push_entry(actor)` → cursor advances (if was_at_last)
4. Prompt assembled → `on_prompt_assembled` calls `finish_sending()`, then `begin_streaming()` → `push_entry(assistant(""))` → cursor advances (if was_at_last)
5. Stream tokens arrive → `append_stream_token()` (no cursor change)
6. Thinking tokens → `begin_thinking()` inserts thinking entry before assistant, shifts indices, advances cursor (if was_at_last)
7. Stream completes → `finish_streaming()`

## Known Bug

The unit test for `push_entry`'s `was_at_last` logic passes. But in production, the cursor doesn't advance across turns. The test doesn't go through the real actor pipeline — it calls methods directly on `ChatSessionState`. The bug likely lives in the async actor pipeline (event ordering, shared state race, or the echo actor's 1-second delay creating a timing window). An `AppWorld` e2e test through the real actor system is needed to reproduce.

**Reproduction**:
- Turn 1: user sends "1", assistant responds "1" — cursor on assistant "1" (CORRECT)
- Turn 2: user sends "2", assistant responds "2" — cursor still on assistant "1" (WRONG, should be on assistant "2")

## Gutter Coloring

- **Yellow** when chat log has focus (Normal or Input scope)
- **Dark gray** when chat log doesn't have focus (Sidebar, Picker, Dashboard)

## Implementation Status

### Committed & Working (chat-scroll phases 1-6)

Commit `583e384537` (last clean commit on `chat-scroll` branch):

- [x] Always-selected invariant
- [x] Smart auto-scroll guard in `push_entry`
- [x] Renderer viewport state feedback
- [x] j/k cursor-aware paging
- [x] Scroll follows cursor (ctrl-d/u, mouse)
- [x] Gutter focus coloring
- [x] Sidebar pin selection jumps to chat log entry
- [x] No cursor wrapping at session boundaries
- [x] Unconditional scroll-to-selected in renderer (flag removed)

### Committed on Fork (97e9757a23, on top of 583e384537)

- [x] j/k advances cursor by exactly 1 on page (not first/last visible)
- [x] `handle_scroll_to_bottom` moves cursor to last entry
- [x] `handle_scroll_to_top` moves cursor to first entry

### Not Working

- [ ] Cursor doesn't follow new messages across turns in production. The `was_at_last` logic is correct in isolation but something in the real actor pipeline breaks it. Needs an e2e test through the real actor system to reproduce.

## Files Modified

| File | Changes |
|------|---------|
| `feat/session/chat_session.rs` | `push_entry` smart guard, `restore_history` auto-select, `begin_thinking` guard, viewport state fields, `visible_entry_range()`, `move_cursor_to_first/last_visible` |
| `feat/ui/chat_log/renderer.rs` | Stores viewport state each frame, gutter focus coloring, unconditional scroll-to-selected |
| `feat/chat_entry_selection/intent.rs` | j/k paging advances by 1, boundary clamping, tall entry tests |
| `feat/chat_entry_selection/validator.rs` | Removed `NoSelection` error variants |
| `feat/chat_input/intent.rs` | `NormalEscape` no longer clears selection |
| `feat/navigation/intent.rs` | Scroll handlers move cursor, scroll-to-top/bottom track cursor |
| `feat/ui/sidebar/pins/pins_section.rs` | Pin selection syncs chat log cursor |
| `feat/intent/handler.rs` | Wired up new intent handlers |
