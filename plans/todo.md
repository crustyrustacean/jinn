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

- [x] Add `pub is_in_flight: bool` to `SessionEntry` in `crates/jinn-session-list/src/model.rs`
- [x] Populate it in the `SessionEntry` literal at `crates/slices/jinn-sidebar/src/sections/sessions/state.rs:121`
- [x] Fix the `SessionEntry` literal in the `crates/jinn-session-list/src/tree.rs` test helper
- [x] Fix the `SessionEntry` literal in `visible_session_at` at `crates/jinn-session-list/src/tree_node.rs`
- [x] Add `is_in_flight: bool` to `SessionListKey`
- [x] Populate it in `SessionListKey::of_session` by reading the cell via `frontend.with_sections`
- [x] Populate it in the `SessionEntry` built by `sorted_open_sessions_split`
- [x] Apply the tint in `assemble_session_line`: restyle every span to `fg(in_flight_fg).bg(in_flight_bg)`, suppressing `Modifier::REVERSED` and the error-red path
- [x] Ensure the tint covers the full row width (indicator, arrow, tree prefix, symbols, title), not just the title span

## Phase 8 — Test the handler marking paths

- [x] `archive_marks_session_in_flight`
- [x] `first_close_press_marks_nothing` and the busy-subtree cases cover the negative paths; a dedicated streaming-session case was redundant with `archive_tree_with_busy_member_marks_nothing` (same `validate_session_close` gate)
- [x] `close_marks_session_in_flight_on_confirm`
- [x] `archive_tree_marks_every_member_of_an_idle_subtree`
- [x] `archive_tree_with_busy_member_marks_nothing`
- [x] `teardown_tree_marks_every_member_of_an_idle_subtree`

## Phase 9 — Test the clearing paths

- [x] `session_removed_clears_in_flight`
- [x] `failed_teardown_clears_in_flight`
- [x] `successful_teardown_keeps_in_flight` (guards the success-clears-too-early regression)
- [x] `archive_failure_clears_in_flight`
- [x] `archive_tree_aborted_by_actor_reports_failure_for_every_member` (store actor)
- [x] `teardown_tree_aborted_by_actor_reports_failure_for_every_member` (lifecycle actor)

### Phase 9 notes

- The two tree-abort tests are deferred to the store/lifecycle actor test modules, since the
  per-member publish helpers live there. They assert observable bus output, not the sidebar cell.

## Phase 10 — Test the render paths

- [x] `in_flight_row_uses_theme_background`
- [x] `in_flight_row_uses_theme_foreground`
- [x] `idle_row_is_not_tinted`
- [x] `selected_and_in_flight_row_is_not_reversed` (tint must not invert)
- [x] `error_and_in_flight_row_is_tinted` (tint takes precedence over error-red)
- [x] `session_list_key_changes_when_a_session_becomes_in_flight`, plus `in_flight_tint_covers_every_span` (guards the render-memo swallow)

## Phase 11 — Verification

- [x] **AC1** `a` on a loaded idle session tints the row immediately, returning to untinted when the session is archived and removed — `archive_marks_session_in_flight` + `session_removed_clears_in_flight`
- [x] **AC2** `x` twice tints the row for the full teardown duration, including across a multi-second teardown script — `close_marks_session_in_flight_on_confirm`; the mark spans the teardown *and* the archive that follows, since only a *failing* teardown clears it (`successful_teardown_keeps_in_flight`)
- [x] **AC3** `A`/`X` twice tints every session in the resolved subtree, not only the selected root — `archive_tree_marks_every_member_of_an_idle_subtree`, `teardown_tree_marks_every_member_of_an_idle_subtree`
- [x] **AC4** pressing `a`/`x`/`A`/`X` on a busy session tints nothing — `archive_tree_with_busy_member_marks_nothing`, `first_close_press_marks_nothing`. All four keys share the `validate_session_close` gate, so the rejection is covered once at the validator rather than four times
- [x] **AC5** `A`/`X` on a subtree with a busy member tints no member and shows the existing `Busy` banner — `archive_tree_with_busy_member_marks_nothing`; the `Err(SubtreeBusy)` arm returns before `emit_tree_command`, and the banner path is untouched
- [x] **AC6** a failed teardown loses its tint — `failed_teardown_clears_in_flight`
- [x] **AC7** a failed archive write loses its tint — `archive_failure_clears_in_flight`
- [x] **AC8** an idle-at-press-time subtree aborted by the actors' re-validation loses **every** member's tint — `archive_tree_aborted_by_actor_reports_failure_for_every_member` (store) and `teardown_tree_aborted_by_actor_reports_failure_for_every_member` (lifecycle); both assert one event per member
- [x] **AC9** a successful teardown does not clear the tint — `successful_teardown_keeps_in_flight`
- [x] **AC10** two concurrent disposals tint both and each clears independently — the set is keyed per `SessionId` and `end_in_flight` removes one id, so this follows structurally from the two tests above. No dedicated test; see divergence note
- [x] **AC11** the tint uses `in_flight_bg`/`in_flight_fg` from the loaded theme, with no literal color in the render path — `in_flight_row_uses_theme_background`, `in_flight_row_uses_theme_foreground`; `entry_line.rs` references only `theme.in_flight_*`
- [x] **AC12** on `nord-light` the tinted row is legible — `light_theme_tint_is_legible_against_its_pale_gutter`
- [x] **AC13** a tinted cursor row and a tinted error row both still read as in-flight — `selected_and_in_flight_row_is_not_reversed`, `error_and_in_flight_row_is_tinted`
- [x] **AC14** `just check` clean, `just lint` zero warnings, `just test` **6508 passed / 0 failed**
- [ ] **Record Updates** — see the divergence note; one planned entry is wrong and needs the user's call before writing

### Divergences from the planned Record Updates

The planned block was written before implementation and one entry no longer matches the code:

- `"(ui) A session's in-flight tint is set when the disposal command is dispatched, so a rejected
  validation leaves no indication."` — **accurate, keep as written.**
- `"(session) An archive that fails in the store actor publishes SessionArchiveFailed, so consumers
  of the archive lifecycle always see a terminal signal."` — **accurate, but understated.** The
  lifecycle actor also publishes `SessionTeardownFinished { error: Some(..) }` on its own tree
  abort, and the store actor publishes per *member*, not per root. Recommend rewording to
  "…publishes SessionArchiveFailed for every affected member…" before writing.
- The theme entry is accurate, with the correction already folded in (a theme omitting the keys
  inherits the **default theme's** values via `resolve_with_fallback`, not `Color::Reset`).

**Nothing has been written to `.agents/RECORD.md` yet** — per the plan, the understated entry is
left for the user to resolve rather than committed as-is.

### Post-completion notes

- `handle_session_tree_action_confirm` remains **dead code** (re-exported, never called). Its
  signature was changed to take `&[SessionId]` so it cannot emit a tree command without tinting.
  Deleting it is a separate call.
- `jinn-session-state` was added as a **dev-dependency** of `jinn-session-lifecycle` purely to
  build the two sessions in the new abort test.
- Three test-authoring mistakes of mine, recorded because they cost real time and shaped the
  final code: `AppState::default()` has no sections cell, so `update_sections` silently drops
  writes and the tests must use `default_with_scope_focus()`; `validate_session_close` also
  requires the sessions section to be pushed onto scope focus, or it rejects before any marking;
  and a `has_live_term: true` blanket edit silently injected an overriding `is_in_flight: false`
  into a struct-update literal, which failed only at the tint assertion.
