# Task List — Archive/Teardown In-Flight Indication

Spec: `.plans/archive-spinner/plan.md`

Status legend: `[ ]` pending · `[x]` done · `[!]` diverged (note the divergence)

---

## Phase 1 — Theme vocabulary

- [x] Add `in_flight_bg` and `in_flight_fg` to the `Theme` struct in `crates/jinn-theme/src/theme.rs`
- [x] Add both keys to the `ThemeEntry` overlay as `Option<ThemeColor>` with `#[serde(default)]`
- [x] Insert both keys into `style_map()` using `Style::default().fg(...)` (fg-only — the existing test asserts it)
- [x] Wire both keys through `resolve_with_fallback`
- [x] Wire both keys through `resolve_standalone`
- [x] Extend the `no_reset_colors` field list in `crates/jinn-theme/src/default_theme.rs`
- [x] Author `in_flight_bg`/`in_flight_fg` in `res/themes/default.toml`
- [x] Author values in `res/themes/catppuccin-mocha.toml`
- [x] Author values in `res/themes/gruvbox-dark.toml`
- [x] Author values in `res/themes/nord-light.toml` — **light theme, so a dark wash with light text**
- [x] Author values in `res/themes/sonokai.toml`
- [x] Update `style_map_returns_entry_for_every_theme_field` count `44` → `46`
- [x] Extend `style_map_values_are_fg_only_styles` with the two new keys
- [x] Add test `every_shipped_theme_defines_in_flight_colors` (all five `res/themes/*.toml` set both keys) — **landed as `shipped_theme_defines_in_flight_colors`**, an rstest case per theme so a failure names the offending file
- [x] Add test `nord_light_in_flight_is_legible` — **landed as `light_theme_tint_is_legible_against_its_pale_gutter`**, asserting the wash differs from `gutter_bg` and the fg survives `ensure_contrast` unchanged

### Phase 1 notes

- Divergence: the spec's §4.1 said to use `Style::default().bg()` for `in_flight_bg`. The existing
  `style_map_values_are_fg_only_styles` test asserts fg-only, so both keys use `.fg()` — matching the
  `selection_bg` precedent. The *render path* (Phase 7) still uses a real `.bg()`.
- The tint is intentionally distinct from `selection_bg` in all five themes: the cursor highlight is
  `Modifier::REVERSED`, and a wash near it would be hard to tell apart from the cursor row.
- Verified: `just check` clean; `style_map_*`, `no_reset_colors`, and both new tests pass (9 tests).

## Phase 2 — Failure event

- [ ] Add `SessionArchiveFailed { session_id, error }` to `crates/jinn-session-msg/src/lib.rs` with serde + `trouper::schema::Event` derives
- [ ] Add `impl jinn_slices::BusMessage for SessionArchiveFailed`
- [ ] Extend the existing wire-contract roundtrip test in `jinn-session-msg` to include it

## Phase 3 — Publish the failure

- [ ] Add a `publish_archive_failed` helper on `SessionStoreActor` publishing one `SessionArchiveFailed` per member
- [ ] Publish at abort: `archive_snapshots` returns `None` (empty snapshot set)
- [ ] Publish at abort: `archive_snapshots` write failure in `archive_members`
- [ ] Publish at abort: a member fails to load in `archive_snapshots`
- [ ] Publish at abort: `guarded_tree_closure` busy re-validation in `SessionStoreActor` — clear **every** member, not just the root
- [ ] Publish at abort: `SessionLifecycleActor::guarded_tree_closure` busy re-validation via `SessionTeardownFinished { error: Some(..) }` for every member

## Phase 4 — In-flight state in the sidebar cell

- [ ] Add `use std::collections::HashSet;` to `crates/slices/jinn-sidebar-msg/src/sidebar_sections.rs`
- [ ] Add `pub in_flight: HashSet<SessionId>` to `SessionsSectionState`
- [ ] Implement `SessionsSectionState::begin_in_flight` (semantic action, not a setter)
- [ ] Implement `SessionsSectionState::end_in_flight` (idempotent no-op when the id is absent)

## Phase 5 — Set the tint at dispatch

- [ ] Add a `mark_in_flight(state, ids)` helper in the sidebar sessions module
- [ ] Call it in `handle_session_archive` after every early return, before building the result
- [ ] Call it in `handle_session_close_with_lifecycle` — **not** in `handle_session_close_arm`'s first press
- [ ] Refactor `archive_tree.rs`: add `emit_tree_command(state, action, &members)` that marks then delegates to `command_for`
- [ ] Route all 3 archive-tree dispatch sites through `emit_tree_command` (lines ~71, ~99, ~131)
- [ ] Confirm every `Err` arm (including `SubtreeBusy`) marks nothing

## Phase 6 — Clear the tint on completion

- [ ] Add `.handles::<SessionArchiveFailed>()` to `SidebarStateActor::spawn`
- [ ] Add `.handles::<SessionTeardownFinished>()` to `SidebarStateActor::spawn`
- [ ] Add a `clear_in_flight` helper using `state.with_session_sidebar(|view| view.frontend.update_sections(...))`
- [ ] Clear the in-flight id in the `MsgHandler<SessionRemoved>` path, alongside the existing cursor clamp
- [ ] Add `MsgHandler<SessionArchiveFailed>` clearing the failed session's id
- [ ] Add `MsgHandler<SessionTeardownFinished>` clearing **only** when `error.is_some()` — success must keep the tint for the archive write that follows

## Phase 7 — Render the tint

- [ ] Add `pub is_in_flight: bool` to `SessionEntry` in `crates/jinn-session-list/src/model.rs`
- [ ] Populate it in the `SessionEntry` literal at `crates/slices/jinn-sidebar/src/sections/sessions/state.rs:121`
- [ ] Fix the `SessionEntry` literal in the `crates/jinn-session-list/src/tree.rs` test helper
- [ ] Fix the `SessionEntry` literal in `visible_session_at` at `crates/jinn-session-list/src/tree_node.rs`
- [ ] Add `is_in_flight: bool` to `SessionListKey`
- [ ] Populate it in `SessionListKey::of_session` by reading the cell via `frontend.with_sections`
- [ ] Populate it in the `SessionEntry` built by `sorted_open_sessions_split`
- [ ] Apply the tint in `assemble_session_line`: restyle every span to `fg(in_flight_fg).bg(in_flight_bg)`, suppressing `Modifier::REVERSED` and the error-red path
- [ ] Ensure the tint covers the full row width (indicator, arrow, tree prefix, symbols, title), not just the title span

## Phase 8 — Test the handler marking paths

- [ ] `archive_marks_session_in_flight`
- [ ] `archive_of_streaming_session_marks_nothing`
- [ ] `close_marks_session_in_flight_on_confirm`
- [ ] `first_close_press_marks_nothing`
- [ ] `archive_tree_marks_every_member_of_an_idle_subtree`
- [ ] `archive_tree_with_busy_member_marks_nothing`
- [ ] `teardown_tree_marks_every_member_of_an_idle_subtree`

## Phase 9 — Test the clearing paths

- [ ] `session_removed_clears_in_flight`
- [ ] `failed_teardown_clears_in_flight`
- [ ] `successful_teardown_keeps_in_flight` (guards the success-clears-too-early regression)
- [ ] `archive_failure_clears_in_flight`
- [ ] `archive_tree_aborted_by_actor_clears_every_member_tint`
- [ ] `teardown_tree_aborted_by_actor_clears_every_member_tint`

## Phase 10 — Test the render paths

- [ ] `in_flight_row_uses_theme_background`
- [ ] `in_flight_row_uses_theme_foreground`
- [ ] `idle_row_is_not_tinted`
- [ ] `selected_and_in_flight_row_is_not_reversed` (tint must not invert)
- [ ] `error_and_in_flight_row_is_tinted` (tint takes precedence over error-red)
- [ ] `session_list_key_rebuilds_on_in_flight_change` (guards the render-memo swallow)

## Phase 11 — Verification

- [ ] **AC1** `a` on a loaded idle session tints the row immediately, and returns to untinted when the session is archived and removed
- [ ] **AC2** `x` pressed twice tints the row for the full teardown duration, including across a multi-second teardown script
- [ ] **AC3** `A` (or `X`) pressed twice tints every session in the resolved subtree, not only the selected root
- [ ] **AC4** pressing any of `a`/`x`/`A`/`X` on a streaming or otherwise busy session tints nothing (validation rejects before dispatch)
- [ ] **AC5** `A`/`X` on a subtree containing any busy member tints no member and shows the existing Busy banner — behavior unchanged
- [ ] **AC6** a session whose teardown fails loses its tint once `SessionTeardownFinished { error: Some(..) }` is published
- [ ] **AC7** a session whose archive write fails loses its tint once `SessionArchiveFailed` is published
- [ ] **AC8** a subtree fully idle at press time but aborted by the actors' re-validation loses every member's tint
- [ ] **AC9** a successful teardown does not clear the tint — the archive write that follows is the visible work
- [ ] **AC10** two concurrent disposals tint both, and each clears independently
- [ ] **AC11** the tint uses `in_flight_bg` / `in_flight_fg` from the loaded theme; no literal color is present in the render path
- [ ] **AC12** on `nord-light`, the tinted row is legible (dark wash, light text)
- [ ] **AC13** a tinted row that is also the cursor row, and a row whose last entry is an error, both still read as in-flight
- [ ] **AC14** `just check`, `just lint`, and `just test` all pass
- [ ] **Record Updates** — review the complete implementation; **(2a)** if it matches the planned Record Updates, write those exact entries into `.agents/RECORD.md`; **(2b)** if it diverged, do **not** write a wrong entry — surface the divergence in the final implementation summary for the user to resolve

### Planned Record Updates (from the spec)

```
- (ui) A sessions sidebar row is tinted with the theme's in-flight colors while its archive or teardown is dispatched and not yet finished.
- (ui) The in-flight indication is per session, not per tree: an archive-tree operation tints each member it resolved and each clears independently.
- (ui) A session's in-flight tint is set when the disposal command is dispatched, so a rejected validation leaves no indication.
- (session) An archive that fails in the store actor publishes SessionArchiveFailed, so consumers of the archive lifecycle always see a terminal signal.
- (theme) Themes carry in_flight_bg and in_flight_fg; a theme that omits them inherits the default theme's values.
```
