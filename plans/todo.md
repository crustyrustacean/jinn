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

- [x] Add `SessionArchiveFailed { session_id, error }` to `crates/jinn-session-msg/src/lib.rs` with serde + `trouper::schema::Event` derives
- [x] Add `impl jinn_slices::BusMessage for SessionArchiveFailed`
- [x] Extend the existing wire-contract roundtrip test in `jinn-session-msg` to include it
- [x] Verified: `session_events_roundtrip_through_json` passes

## Phase 3 — Publish the failure

- [x] Add a `publish_archive_failed` helper on `SessionStoreActor` publishing one `SessionArchiveFailed` per member
- [x] Publish when `archive_members` gets no snapshots (covers the member-load-failure abort inside `archive_snapshots`, which propagates as `None`)
- [x] Publish when the `archive_snapshots` store write fails in `archive_members`
- [x] Publish in `guarded_tree_closure` on the busy re-validation — clears **every** member, not just the root
- [x] Publish in `SessionLifecycleActor::guarded_tree_closure` on the busy re-validation via `SessionTeardownFinished { error: Some(..) }` for every member
- [x] `just check` clean

### Phase 3 notes

- Simplification: the spec listed five abort sites (1–5). `archive_snapshots` has three internal
  abort paths, but the member-load failure `return None`s to `archive_members`, and the empty-snapshot
  `then_some(None)` does too. Publishing once at the caller covers all three, so no publish was added
  inside the helper.
- Compile error hit and fixed: adding an `await` to `guarded_tree_closure` made the
  `StateReadGuard` cross the await point, failing `Send` for the `MsgHandler` future. The explicit
  `drop(state)` was not provable to the compiler. Replaced with a block-scoped `let busy = { ... }`,
  which is also what the style guide's block-scoping rule asks for. Applied to both actors.

## Phase 4 — In-flight state in the sidebar cell

- [x] Add `use std::collections::HashSet;` to `crates/slices/jinn-sidebar-msg/src/sidebar_sections.rs`
- [x] Add `pub in_flight: HashSet<SessionId>` to `SessionsSectionState`
- [x] Implement `SessionsSectionState::begin_in_flight` (semantic action, not a setter)
- [x] Implement `SessionsSectionState::end_in_flight` (idempotent no-op when the id is absent)
- [x] Implement `SessionsSectionState::is_in_flight` — a read accessor so the render path asks a question rather than poking the set
- [x] `just check` clean — the struct derives `Default`, so no existing literals broke

## Phase 5 — Set the tint at dispatch

- [x] Add a `mark_in_flight(state, ids)` helper in `sessions/state.rs`, plus `clear_in_flight` / `is_in_flight` for the clearing and render phases
- [x] Call it in `handle_session_archive` after every early return, before building the result
- [x] Call it in `handle_session_close_with_lifecycle` — the first `x` press only arms the prompt and marks nothing
- [x] Add `emit_tree_command(state, action, &members)` in `archive_tree.rs` that marks then delegates to `command_for`
- [x] Route both tree dispatch sites (lines 71, 99) through `emit_tree_command`
- [x] Confirm every `Err` arm (including `SubtreeBusy`) marks nothing — it returns before `emit_tree_command`
- [x] `just check` clean

### Phase 5 notes

- Only **two** tree dispatch sites call `command_for`, not the three the spec predicted. The third
  (`handle_session_tree_action_confirm`) is dead code: re-exported from `sessions.rs` but called
  nowhere in the repo. Its signature also took only a `root`, so it could not know the member list.
  Changed it to take `&[SessionId]` and route it through `emit_tree_command`, so if it is ever wired
  up it cannot emit a command that leaves the tint unset. It is still unused — flagged for the user
  rather than deleted, since removing public API is out of scope here.
- The spec predicted 3 or 4 call sites; routing all of them through one `emit_tree_command` is what
  makes the invariant hold, so the count mattered less than the consolidation.
- Fixed an authoring slip: `with_sections` takes a fallback *thunk*, not a value (`|| false`, not `false`).

## Phase 6 — Clear the tint on completion

- [x] Add `.handles::<SessionArchiveFailed>()` to `SidebarStateActor::spawn`
- [x] Add `.handles::<SessionTeardownFinished>()` to `SidebarStateActor::spawn`
- [x] Clear the in-flight id in the `SessionRemoved` path, alongside the existing cursor clamp
- [x] Add `MsgHandler<SessionArchiveFailed>` clearing the failed session's id
- [x] Add `MsgHandler<SessionTeardownFinished>` clearing **only** when `error.is_some()`

### Phase 6 notes

- The spec said to verify whether `with_session_sidebar`'s `view.frontend` reaches the sections
  cell. It does — `clear_in_flight(view.frontend, ..)` works unchanged, so no fallback to a
  direct cell write was needed.
- Clippy bans `impl Trait` in parameter position (`-D clippy::impl-trait-in-params`), so
  `begin_in_flight` takes `&[SessionId]` rather than `impl IntoIterator<Item = SessionId>`. Every
  caller already had a slice. Committed as a follow-up fix after the first Phase 4-5 commit had
  already gone in with a lint failure — flagging that ordering slip rather than hiding it.

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

- [x] `session_removed_clears_in_flight`
- [x] `failed_teardown_clears_in_flight`
- [x] `successful_teardown_keeps_in_flight` (guards the success-clears-too-early regression)
- [x] `archive_failure_clears_in_flight`
- [ ] `archive_tree_aborted_by_actor_clears_every_member_tint`
- [ ] `teardown_tree_aborted_by_actor_clears_every_member_tint`

### Phase 9 notes

- The two tree-abort tests are deferred to the store/lifecycle actor test modules, since the
  per-member publish helpers live there. They assert observable bus output, not the sidebar cell.

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
