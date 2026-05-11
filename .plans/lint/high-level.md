# Lint: Fix all clippy/compiler warnings

## Problem

`just lint` produces 12 clippy/compiler warnings across the workspace. These fall into four categories:

1. **Redundant import** — `use kanal;` in `nsslice-llm/src/lib.rs:13`
2. **Dead code** — unused `test_services()` function in `nullslop-component/src/lib.rs:63` and an unfulfilled `#[expect(dead_code)]` in `nullslop-component/src/chat_session/state.rs:310`
3. **Missing docs on private items** — 8 functions across 3 crates lack `///` doc comments (enforced by `-W clippy::missing-docs-in-private-items`)
4. **Unused import** — `nullslop_protocol::context::PinChatEntry` in `nsslice-pinned-panel/src/intent.rs:148`

Test module length warnings (26 modules > 200 lines) are **out of scope** for this task.

## Acceptance Criteria

- `just lint` exits with zero warnings (excluding test-length warnings which are out of scope)
- No behavioral changes — only lint fixes
- All existing tests continue to pass

## Plan

- [ ] Phase 1: Fix all clippy/compiler warnings
  - [ ] Remove redundant `use kanal;` from `crates/slices/nsslice-llm/src/lib.rs:13`
  - [ ] Remove unused `test_services()` function from `crates/common/nullslop-component/src/lib.rs` (lines 62-65 inside the `#[cfg(test)] mod test_utils` block)
  - [ ] Fix unfulfilled `#[expect(dead_code)]` on `finalize_tool_call` in `crates/common/nullslop-component/src/chat_session/state.rs:310` — change to `#[cfg(test)]` attribute since the function is only used in tests (the `#[expect(dead_code)]` was unfulfilled because the function IS used, just only in test builds; a `#[cfg(test)]` pub(crate) fn doesn't need dead_code suppression)
  - [ ] Add `///` doc comments to 3 private functions in `crates/slices/nsslice-chat-input-box/src/intent.rs`:
    - `is_valid_trigger_position` (line 319)
    - `should_deactivate_on_cursor_move` (line 327)
    - `compute_matches` (line 334)
  - [ ] Add `///` doc comments to 2 private functions in `crates/slices/nsslice-pinned-panel/src/intent.rs`:
    - `resolve_selected_entry_id` (line 125)
    - `cycle_position` (line 137)
  - [ ] Add `///` doc comments to 3 private functions in `crates/slices/nsslice-picker/src/intent.rs`:
    - `confirm_provider` (line 218)
    - `confirm_strategy` (line 233)
    - `confirm_session` (line 263)
  - [ ] Remove unused import `nullslop_protocol::context::PinChatEntry` from `crates/slices/nsslice-pinned-panel/src/intent.rs:148`
  - [ ] Run `just lint` and verify zero clippy/compiler warnings
  - [ ] Run `just test` and verify all tests pass
