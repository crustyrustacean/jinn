//! Tests for the project-add popup actions and hook.

#![allow(
    clippy::expect_used,
    clippy::field_reassign_with_default,
    clippy::panic,
    reason = "test code"
)]

use super::intent::{
    attach_project_add_rows, clear_or_leave_project_add, confirm_project_add, leave_project_add,
    open_project_add, project_add_scope, project_add_slot, register_project_add_input_hook,
};
use super::state::ProjectAddInputState;
use jinn_slices::KeyRoutes;
use jinn_slices::PublishClosure;
use jinn_slices::SliceActionState;
use jinn_slices::TypedCell;
use jinn_slices::route::ActionCtx;
use jinn_slices::route::BindSite;
use jinn_slices::route::EditIntent;
use jinn_slices::route::ScopeSignal;

/// A minimal [`SliceActionState`] double that also carries kernel state
/// for the optimistic-write downcast.
struct FakeState {
    cwd: std::path::PathBuf,
    kernel: Option<jinn_domain::AppState>,
}

impl Default for FakeState {
    fn default() -> Self {
        Self {
            cwd: std::path::PathBuf::new(),
            kernel: Some(jinn_domain::AppState::default_with_scope_focus()),
        }
    }
}

impl SliceActionState for FakeState {
    fn active_session_title(&self) -> Option<String> {
        None
    }

    fn active_session_id(&self) -> jinn_core_types::SessionId {
        self.kernel
            .as_ref()
            .expect("kernel state present")
            .session
            .active_session_id()
            .clone()
    }

    fn push_session_error(&mut self, _message: &str) {}

    fn active_session_cwd(&self) -> std::path::PathBuf {
        self.cwd.clone()
    }

    fn publish_session_cwd(
        &self,
        _session_id: jinn_core_types::SessionId,
        _cwd: std::path::PathBuf,
    ) -> PublishClosure {
        Box::new(|_bus| {})
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        // Mirror the kernel impl: lend the concrete AppState by type.
        self.kernel.as_mut().map(|k| k as &mut dyn std::any::Any)
    }
}

fn ctx<'a>(state: &'a mut FakeState, slices: &'a jinn_slices::Slices) -> ActionCtx<'a> {
    ActionCtx {
        state,
        slices,
        key_bytes: Vec::new(),
    }
}

/// Mints the popup cell in a fresh registry; returns both.
fn cell() -> (jinn_slices::Slices, TypedCell<ProjectAddInputState>) {
    let slices = jinn_slices::Slices::new();
    let cell = slices
        .register(project_add_slot(), ProjectAddInputState::default())
        .expect("fresh test registry has the slot free");
    (slices, cell)
}

#[rstest::rstest]
fn rows_bind_confirm_leave_clear_and_open() {
    // Given a route table with the popup rows attached.
    let routes = KeyRoutes::new();
    let (_slices, cell) = cell();
    attach_project_add_rows(&routes, &cell);

    // When enumerating the rows.
    let rows = routes.rows();

    // Then the opener binds <c-n> on the static project-picker scope.
    let open = rows
        .iter()
        .find(|row| row.route_id.as_str() == "project-add:open")
        .expect("opener row");
    assert_eq!(open.key, "<c-n>");
    assert!(matches!(open.site, BindSite::StaticScopes(_)));
    // And confirm/leave/clear bind in the popup's own scope.
    let confirm = rows
        .iter()
        .find(|row| row.route_id.as_str() == "confirm-project-add")
        .expect("confirm row");
    assert_eq!(confirm.key, "<enter>");
    assert!(matches!(confirm.site, BindSite::OwnScope));
    let leave = rows
        .iter()
        .find(|row| row.route_id.as_str() == "leave-project-add")
        .expect("leave row");
    assert_eq!(leave.key, "<esc>");
    let clear = rows
        .iter()
        .find(|row| row.route_id.as_str() == "clear-or-leave-project-add")
        .expect("clear row");
    assert_eq!(clear.key, "<c-c>");
}

#[rstest::rstest]
fn open_seeds_input_from_session_cwd_and_pushes_scope() {
    // Given a fake state whose session cwd is an absolute path.
    let mut state = FakeState::default();
    state.cwd = std::path::PathBuf::from("/tmp/some-project");
    let (slices, cell) = cell();
    let cx = ctx(&mut state, &slices);

    // When the opener action runs.
    let result = open_project_add(cx.state, &cell);

    // Then the cell is seeded with the cwd and the result pushes the
    // popup scope.
    assert_eq!(cell.read().text.input, "/tmp/some-project");
    assert_eq!(cell.read().text.cursor_pos, "/tmp/some-project".len());
    assert!(matches!(result.scope_signal, Some(ScopeSignal::Push(_))));
}

#[rstest::rstest]
fn confirm_valid_dir_appends_project_optimistically_and_emits_update() {
    // Given a real tempdir next to the session cwd, with the popup seeded
    // with the tempdir's basename.
    let temp = tempfile::tempdir().expect("tempdir");
    let target = temp.path();
    let mut state = FakeState::default();
    state.cwd = target.parent().expect("parent").to_path_buf();
    let (slices, cell) = cell();
    cell.update(|s| {
        s.text.set(
            target
                .file_name()
                .expect("name")
                .to_string_lossy()
                .to_string(),
        );
    });
    let mut cx = ctx(&mut state, &slices);

    // When confirming.
    let result = confirm_project_add(&mut cx, &cell);

    // Then one UpdatePreferences is published and the scope pops.
    assert_eq!(result.message_names, vec!["UpdatePreferences"]);
    assert_eq!(result.messages.len(), 1);
    assert!(matches!(result.scope_signal, Some(ScopeSignal::PopIf(_))));
    // And the optimistic write appended the project to kernel state.
    let kernel = state.kernel.as_ref().expect("kernel state present");
    assert_eq!(kernel.frontend.preferences.projects.len(), 1);
    // And the cell is cleared.
    assert_eq!(cell.read().text.input, "");
}

#[rstest::rstest]
fn confirm_invalid_dir_stays_open_without_publishing() {
    // Given a popup seeded with a path that is not a directory.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut state = FakeState::default();
    state.cwd = temp.path().to_path_buf();
    let (slices, cell) = cell();
    cell.update(|s| s.text.set("no-such-dir".to_owned()));
    let mut cx = ctx(&mut state, &slices);

    // When confirming.
    let result = confirm_project_add(&mut cx, &cell);

    // Then nothing is published and no scope transition happens.
    assert!(result.messages.is_empty());
    assert!(result.scope_signal.is_none());
    // And the input is preserved for correction.
    assert_eq!(cell.read().text.input, "no-such-dir");
    // And kernel state gained no project.
    let kernel = state.kernel.as_ref().expect("kernel state present");
    assert!(kernel.frontend.preferences.projects.is_empty());
}

#[rstest::rstest]
fn clear_or_leave_with_text_clears_and_stays_open() {
    // Given a popup with typed text.
    let (_slices, cell) = cell();
    cell.update(|s| s.text.set("/tmp/some-project".to_owned()));

    // When Ctrl-C is requested.
    let result = clear_or_leave_project_add(&cell);

    // Then the input clears without leaving the popup.
    assert!(result.scope_signal.is_none());
    assert!(cell.read().text.input.is_empty());
}

#[rstest::rstest]
fn clear_or_leave_with_empty_text_clears_and_leaves() {
    // Given an empty popup.
    let (_slices, cell) = cell();

    // When Ctrl-C is requested.
    let result = clear_or_leave_project_add(&cell);

    // Then the popup requests its own conditional pop.
    assert_eq!(
        result.scope_signal,
        Some(ScopeSignal::PopIf(project_add_scope()))
    );
    assert!(cell.read().text.input.is_empty());
}

#[rstest::rstest]
fn leave_clears_the_cell() {
    // Given a popup with text.
    let (_slices, cell) = cell();
    cell.update(|s| s.text.set("/tmp/some-project".to_owned()));

    // When leaving.
    leave_project_add(&cell);

    // Then the cell is cleared.
    assert_eq!(cell.read().text.input, "");
}

#[rstest::rstest]
fn input_hook_edits_the_cell() {
    // Given a route table with the input hook registered.
    let routes = KeyRoutes::new();
    let (_slices, cell) = cell();
    register_project_add_input_hook(&routes, &cell);
    let hook = routes
        .input_hook(&project_add_scope())
        .expect("hook registered for the popup scope");

    // When editing intents flow through the hook.
    let _ = hook(&EditIntent::InsertChar('/'));
    let _ = hook(&EditIntent::InsertChar('t'));
    let _ = hook(&EditIntent::CursorLeft);
    let _ = hook(&EditIntent::InsertChar('m'));

    // Then the cell reflects the edits: the 'm' landed before the 't'.
    let state = cell.read();
    assert_eq!(state.text.input, "/mt");
}

#[rstest::rstest]
fn input_hook_moves_cursor_home_and_consumes_intent() {
    // Given a route table with the input hook registered and text.
    let routes = KeyRoutes::new();
    let (_slices, cell) = cell();
    cell.update(|s| s.text.set("héllo".to_owned()));
    register_project_add_input_hook(&routes, &cell);
    let hook = routes
        .input_hook(&project_add_scope())
        .expect("hook registered for the popup scope");

    // When moving the cursor home.
    let result = hook(&EditIntent::CursorHome);

    // Then the intent is consumed and the cursor reaches the start.
    assert!(result.is_some());
    assert_eq!(cell.read().text.cursor_pos, 0);
}

#[rstest::rstest]
fn input_hook_moves_cursor_end_and_consumes_intent() {
    // Given a route table with the input hook registered and text.
    let routes = KeyRoutes::new();
    let (_slices, cell) = cell();
    cell.update(|s| s.text.set("héllo".to_owned()));
    cell.update(|s| s.text.cursor_home());
    register_project_add_input_hook(&routes, &cell);
    let hook = routes
        .input_hook(&project_add_scope())
        .expect("hook registered for the popup scope");

    // When moving the cursor to the end.
    let result = hook(&EditIntent::CursorEnd);

    // Then the intent is consumed and the cursor reaches the end.
    assert!(result.is_some());
    assert_eq!(cell.read().text.cursor_pos, "héllo".len());
}
