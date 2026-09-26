# Task List — lifecycle-picker-reset

Spec: `.plans/lifecycle-picker-reset/plan.md`

Phases mirror the approved plan. The final **Verification** phase carries each
acceptance criterion as its own task, plus the Record Updates task.

---

## Phase 1 — jinn-slices: the scope-enter hook seam

- [x] Add the `ScopeEnterHook` type alias in `crates/jinn-slices/src/route.rs`, after the `InputHook`/`KeyHook` aliases, documenting that it fires on scope entry (not per keystroke) and returns nothing
- [x] Add the `scope_enter_hooks: row_store::HookStore<ScopeEnterHook>` field to the `KeyRoutes` struct
- [x] Add `register_scope_enter_hook` / `scope_enter_hook` / `scope_enter_hook_scopes` methods to `KeyRoutes`, mirroring the input-hook block
- [x] Re-export `ScopeEnterHook` from `crates/jinn-slices/src/lib.rs` (alphabetical, between `RouteRow` and `ScopeSignal`)
- [x] Extend the module doc at the top of `route.rs` with a paragraph on the scope-enter hook as the per-open-state initializer
- [x] Add the `jinn-slices` unit test: register a hook, assert lookup returns `Some`, assert an unregistered scope returns `None`

**Note:** `row_store` is a private inline module in `route.rs`, not a separate file — the plan's import
paths read as `row_store::` from inside `route.rs`, which is exactly how it resolved. `HookStore::get`
requires `H: Clone`; `Arc<dyn Fn...>` satisfies it for free. Verified `key_bytes: Vec<u8>` (not
`Vec<String>`). `just check` clean; 3 new tests pass.

## Phase 2 — Kernel: fire the enter hook on push

- [x] Widen `apply_scope_signal` in `crates/jinn-domain/src/feat/intent/handler.rs` to take `slices`, `routes`, and `config`
- [x] Fire the registered scope-enter hook on `ScopeSignal::Push`, after the scope push so the scope is active; leave `PopIf` unchanged
- [x] Update the single call site (`handler.rs:290`) to pass `slices`, `routes`, and `config`; resolve any reborrow issue on `state`
- [x] Update the `apply_scope_signal` doc comment to mention the enter hook
- [x] Add handler test: a `Push` transition fires the target scope's enter hook
- [x] Add handler test: a `Push` transition leaves the scope on the stack
- [x] Add handler test: a `PopIf` transition fires no enter hook

**Notes:**
- The plan's predicted reborrow problem on `state` **did not occur** — `state` is consumed by the
  `ActionCtx` literal inside the `Push` arm and the function returns right after. No fix needed.
- The plan's combined "Push fires the hook *and* leaves the scope on the stack" test was **split into
  two** per one-test-one-behavior (`AGENTS.md` §4): firing the hook and pushing the scope are two
  observable behaviors, and a failure in one shouldn't be reported as the other.
- Added `use crate::protocol::ScopeSignal;` to the test module. **`just check` did not catch this** —
  it compiles lib targets only, so test-only code needs `just test-one` to be compile-gated.

## Phase 3 — Lifecycle picker: seed on enter

- [x] Add `register_session_lifecycle_picker_enter_hook` in `session_lifecycle_picker_routes.rs`, reading the config before downcasting app state for the theme
- [x] Use `jinn_theme::default_theme()` as the theme fallback when the `app(ctx)` downcast fails (`Theme` has no `Default` impl — re-confirmed at `crates/jinn-theme/src/theme.rs`, only `default_theme()` exists)
- [x] Reduce `open_session_lifecycle_picker` to a bare `ScopeSignal::Push` with unused ctx/cell params; `SessionLifecycle` import still needed by the enter hook and `confirm_session_lifecycle_picker`
- [x] Call the new registration from `activate_picker` in `crates/slices/jinn-session-lifecycle/src/lib.rs`
- [x] Update the doc on `session_lifecycle_picker_actions::open` to name the enter hook as its caller
- [x] Fix `Wired::open()` in `session_lifecycle_picker_tests.rs` to apply the scope signal (push + invoke the enter hook), mirroring production — **highest-risk edit**
- [x] Add test: reopening the picker after typing a filter leaves the filter empty
- [x] Add test: reopening moves the highlight back to the first row
- [x] Verify the existing test `opening_the_picker_lists_every_configured_lifecycle_after_blank` still passes unmodified in intent
- [x] Add cross-slice regression test in `tests/slices/session_lifecycle.rs`: project `<c-enter>` populates the lifecycle picker with blank + configured lifecycles, exercising the scope-push path
- [x] Add cross-slice test in `tests/slices/session_lifecycle.rs`: project `<c-enter>` clears a stale filter typed during a previous visit

**Notes:**
- `Wired::open()` was **split into two helpers** rather than one, because several existing tests
  (`opening_the_picker_pushes_its_own_scope` and the confirm/cancel cases) read
  `result.scope_signal` off `open()`'s return value. Applying the signal inside `open()` would
  `take()` that field and break them. Now: `fire()` dispatches and leaves the signal intact;
  `apply_signal()` consumes one; `open()` = fire + apply.
- All 28 pre-existing picker tests pass unmodified; the two new reset tests pass and each asserts
  its "Given" precondition first so neither can pass vacuous.
- The cross-slice tests drive **`IntentHandler::handle`**, not `routes.action_for`, because the
  kernel is the only place a scope signal is applied and an enter hook fires. Dispatching the
  action directly would skip the very mechanism under test.
- The config key is `[[project.entry]]`, not `[[project]]` (confirmed at
  `crates/jinn-preferences-config/src/schemas/project.rs:17`).
- **Both cross-slice tests were verified to fail without the hook** — temporarily disabling
  `register_session_lifecycle_picker_enter_hook` reproduces the reported symptoms exactly
  (`left: []` vs `right: ["blank","dev"]`, and a stale filter of `"z"`). They are not vacuous.
- Config, state, and route tables are shared across both slices in one `CrossSlice` harness, since
  the two openers only meet in production through one route table and one cell registry.

## Phase 4 — Remove the dead `N` row

- [ ] Delete the `session-new-lifecycle` / `N` / "new session (setup)" route row from `crates/slices/jinn-sidebar/src/key_routes.rs`, keeping the `n` row
- [ ] Remove the `N` row from the General/app table in `res/skills/jinn-usage/references/keybindings.md`
- [ ] Change the sessions-section `n` / `N` table row in `keybindings.md` to `n` only
- [ ] Grep `res/skills/jinn-usage/` for any other sessions-context `N` mention and fix if found
- [ ] Add test in `tests/slices/composition.rs` asserting the sidebar sessions scope binds no `N` key

## Phase 5 — Verification

- [ ] **Acceptance:** project picker `<c-enter>` shows every configured lifecycle plus `blank` on a boot where the picker was never opened
- [ ] **Acceptance:** both entry points open with an empty filter box; a filter typed during a previous visit is gone
- [ ] **Acceptance:** both entry points open on the same state (blank highlighted, full row list)
- [ ] **Acceptance:** a picker whose slice registered no scope-enter hook behaves exactly as before
- [ ] **Acceptance:** `N` in the sidebar sessions section no longer resolves, and the bundled keybindings reference no longer documents it
- [ ] Run `just check`, `just test`, and `just lint` until all pass (never `cargo test` directly; use `just test-one` to iterate)
- [ ] **Record Updates:** review the complete implementation; if it matches the planned entry, write into `.agents/RECORD.md`:
      `- (pickers) A slice picker resets its per-open state — filter text, highlight, and rows — each time its dynamic scope is entered, so every opener shows the same fresh menu.`
      If the implementation **diverged** from that entry, do **not** write a wrong entry — surface the divergence in the final implementation summary for the user to resolve.
