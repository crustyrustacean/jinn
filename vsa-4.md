# VSA-4: Detour

## Context from VSA-3 discussion

After VSA-3 extracts Navigation, Session & Model, and a global app slice, `nullslop-intent` will still have a few remaining items. VSA-4 handles these final extractions and protocol-level cleanups.

## Items

### 4. `Interrupt` gains optional `session_id` field

Currently `Interrupt` has no fields — it reads `state.session.active_session` at handle time ("whatever the user is looking at right now"). The intent is to add:

```rust
Interrupt {
    /// If None, targets the active session. If Some, targets a specific session.
    session_id: Option<SessionId>,
}
```

This enables programmatic interrupt from anywhere (e.g., actor system canceling a session that isn't visible). The `None` path preserves current behavior.

### 5. `NormalEscape` → `nsslice-chat-input-box` + fix pinned pane bug

Move `NormalEscape` handler into `nsslice-chat-input-box`. Its current handler:

1. Validates (infallible)
2. Clears chat entry selection
3. Sets `pinned_pane_close` signal ← **this is a bug**

Step 3 is wrong — NormalEscape shouldn't close the pinned pane. The pinned panel has its own close intent (`PinnedPanelClose`). This bug should be fixed as part of the move.

The handler also clears chat entry selection, which is consistent with chat-input-box's domain (it's the "escape from typing/selection back to neutral" action).

### 6. `SetMode` split into `EnterInsertMode` + `EnterNormalMode`

Current `SetMode { mode: Mode }` does too much for one intent:

- Input → Normal + streaming: cancels stream, drains queue, emits CancelStream
- Picker → non-Picker: clears active_picker_kind
- Always: sets the mode field

Split into two focused intents:

**`EnterInsertMode`** — transitions from Normal to Input mode. Simple: just set the mode.

**`EnterNormalMode`** — transitions from Input/Picker/whatever to Normal mode:

- If currently in Input mode AND session is streaming → cancel stream + drain queue + emit CancelStream
- If currently in Picker mode → clear `active_picker_kind`
- Set mode to Normal

This is cleaner because each intent has a clear precondition and side effect set, rather than a generic mode setter with conditional branching.

### 7. After all extracted: delete validators/ directory entirely

Once all validators have been moved to their respective slices, `nullslop-intent/src/validators/` will be empty. Delete `app.rs` + `mod.rs` + the directory.

## After VSA-4

`nullslop-intent` becomes a pure dispatch hub: `tui_signals.clear()` + `match intent { ... }` where every arm delegates to a slice crate. Plus the picker re-dispatch logic for keymap confirm.

## Open questions for planning

1. How `Interrupt { session_id: Some(id) }` should behave — full interrupt logic or just CancelStream?
2. Does `EnterNormalMode` need to handle the Picker → Normal cleanup, or is that already handled elsewhere?
