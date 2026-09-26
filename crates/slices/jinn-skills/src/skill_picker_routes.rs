//! The skill picker's route rows and input hook.
//!
//! This is where the skill picker becomes fully slice-owned. Every key the
//! picker responds to is a [`RouteRow`] the skills slice attaches itself; the
//! kernel contributes no keybind, no scope variant, and no picker identifier.
//!
//! Two shapes are worth knowing:
//!
//! - **Actions** carry a closure that runs against the picker's own cell, and
//!   reach session state only through [`SliceActionState::as_any_mut`] — the
//!   same seam `jinn-sidebar`, `jinn-term`, `jinn-project`,
//!   `jinn-session-lifecycle`, and `jinn-preferences` already use.
//! - **The filter** is an *input hook* rather than rows, because
//!   `RouteRow` has no catch-all variant. Registering the hook makes the
//!   composition keymap synthesize the printable-character catch-all and the
//!   editing keys for this scope automatically, so typing keeps working.
//!
//! A cell guard is never held across an await: every action snapshots what it
//! needs, mutates, and drops the guard before it returns.

use std::collections::HashSet;
use std::sync::Arc;

use jinn_core_types::{ChatEntry, ChatEntryId, PinPosition, ToolResultStatus};
use jinn_skills_msg::{Skill, SkillPickerState};
use jinn_slices::KeyRoutes;
use jinn_slices::RouteId;
use jinn_slices::RouteResult as IntentResult;
use jinn_slices::cell::TypedCell;
use jinn_slices::route::{
    ActionCtx, ActionFn, BindSite, EditIntent, InputHook, RouteOutcome, RouteRow, ScopeSignal,
};

use crate::skill_picker_actions;
use crate::skill_picker_scope::skill_picker_scope;

/// Rows of preview lines revealed or hidden per `<c-d>` / `<c-u>` press.
const PREVIEW_PAGE_SIZE: usize = 10;

/// The picker's cell — the single home for everything it shows.
type SkillPickerCell = TypedCell<SkillPickerState>;

/// The picker's rows as data: the keys it binds, in footer order.
///
/// The footer renders from this, and a test asserts every entry here is a key
/// the picker actually binds — a footer that advertises a dead key is a bug.
pub const SKILL_PICKER_BINDINGS: &[(&str, &str)] = &[
    ("<tab>", "toggle"),
    ("<c-l>", "load"),
    ("<c-r>", "refresh"),
    ("<c-u>", "preview up"),
    ("<c-d>", "preview down"),
    ("<enter>", "apply"),
    ("<esc>", "cancel"),
];

/// Every key the picker binds in its own scope, for the wiring test.
#[must_use]
pub fn bound_keys() -> Vec<&'static str> {
    SKILL_PICKER_BINDINGS.iter().map(|(key, _)| *key).collect()
}

/// The kernel's application state behind an [`ActionCtx`].
///
/// The picker's open, confirm, and load actions must change session state, so
/// they downcast. This is the established slice-side seam, documented on
/// [`SliceActionState::as_any_mut`]; when the state is not the kernel's (a test
/// double), the caller gets `None` and the action declines rather than panicking.
fn app<'a>(ctx: &'a mut ActionCtx<'_>) -> Option<&'a mut jinn_domain::AppState> {
    ctx.state
        .as_any_mut()?
        .downcast_mut::<jinn_domain::AppState>()
}

/// Wraps a picker action in an [`ActionFn`], handing it both the dispatch
/// context and the cell.
fn action<F>(cell: &SkillPickerCell, f: F) -> ActionFn
where
    F: Fn(&mut ActionCtx<'_>, &SkillPickerCell) -> IntentResult + Send + Sync + 'static,
{
    let cell = cell.clone();
    ActionFn::new(move |mut ctx| f(&mut ctx, &cell))
}

/// Builds one `Action` row binding `key` in the picker's own scope.
fn row(
    action_name: &'static str,
    key: &'static str,
    category: &'static str,
    display: &'static str,
    run: ActionFn,
) -> RouteRow {
    RouteRow {
        route_id: RouteId::new(action_name),
        scope: skill_picker_scope(),
        key,
        category,
        site: BindSite::OwnScope,
        feature: "skills",
        outcome: RouteOutcome::Action {
            action: action_name,
            display,
            run,
        },
    }
}

/// Attaches every row the skill picker owns.
///
/// The opener binds in the `Normal` static scope rather than the picker's own:
/// a key that opens the picker cannot live inside the scope it opens. The
/// remaining rows are `OwnScope`, so they bind only while the picker is on top
/// and cannot shadow another slice's keys.
pub fn attach_skill_picker_rows(routes: &KeyRoutes, cell: &SkillPickerCell) {
    routes.attach(RouteRow {
        route_id: RouteId::new("skills:open"),
        scope: skill_picker_scope(),
        key: "<leader>sk",
        category: "general",
        site: BindSite::StaticScopes(&["Normal"]),
        feature: "skills",
        outcome: RouteOutcome::Action {
            action: "open-skill-picker",
            display: "search skills",
            run: action(cell, open_skill_picker),
        },
    });

    routes.attach(row(
        "toggle-highlighted-skill",
        "<tab>",
        "input",
        "toggle the highlighted skill",
        action(cell, toggle_highlighted_skill),
    ));
    routes.attach(row(
        "load-highlighted-skill",
        "<c-l>",
        "input",
        "load the highlighted skill into context",
        action(cell, load_highlighted_skill),
    ));
    routes.attach(row(
        "refresh-skill-resources",
        "<c-r>",
        "general",
        "rescan skills, prompt templates, and context files",
        action(cell, refresh_skill_resources),
    ));
    routes.attach(row(
        "scroll-skill-preview-up",
        "<c-u>",
        "navigation",
        "scroll the preview up",
        action(cell, scroll_preview_up),
    ));
    routes.attach(row(
        "scroll-skill-preview-down",
        "<c-d>",
        "navigation",
        "scroll the preview down",
        action(cell, scroll_preview_down),
    ));
    routes.attach(row(
        "confirm-skill-picker",
        "<enter>",
        "general",
        "apply the toggled skills and close",
        action(cell, confirm_skill_picker),
    ));
    routes.attach(row(
        "cancel-skill-picker",
        "<esc>",
        "general",
        "close without changing which skills are enabled",
        action(cell, cancel_skill_picker),
    ));
    routes.attach(row(
        "new-session-from-skill-picker",
        "<c-n>",
        "general",
        "start a new session",
        action(cell, |ctx, _cell| {
            let Some(state) = app(ctx) else {
                return IntentResult::empty();
            };
            jinn_domain::feat::session::intent::handle_session_new(state)
        }),
    ));
    routes.attach(row(
        "clear-filter-or-leave-skill-picker",
        "<c-c>",
        "general",
        "clear the filter, or close when already empty",
        action(cell, clear_filter_or_leave),
    ));

    attach_navigation_rows(routes, cell);
}

/// Attaches the four list-navigation rows.
///
/// They cannot be `StaticIntent` rows: composition's `static_intent` table
/// knows only six route ids, none of them picker intents, and a row naming an
/// unknown id is silently dropped with a warning. So the picker implements its
/// own navigation, which also lets it reset the preview scroll on a selection
/// change — the behavior the spec declared as
/// `reset_scroll_on_selection_change`.
fn attach_navigation_rows(routes: &KeyRoutes, cell: &SkillPickerCell) {
    for (name, key, display, step) in [
        ("move-skill-picker-up", "<up>", "move up", Nav::Up),
        ("move-skill-picker-down", "<down>", "move down", Nav::Down),
        ("page-skill-picker-up", "<pgup>", "page up", Nav::PageUp),
        (
            "page-skill-picker-down",
            "<pgdn>",
            "page down",
            Nav::PageDown,
        ),
    ] {
        routes.attach(row(
            name,
            key,
            "navigation",
            display,
            action(cell, move |ctx, cell| {
                navigate(ctx, cell, step);
                IntentResult::empty()
            }),
        ));
    }
}

/// Which list-navigation key was pressed.
#[derive(Debug, Clone, Copy)]
enum Nav {
    /// `<up>` — one row.
    Up,
    /// `<down>` — one row.
    Down,
    /// `<pgup>` — half the visible window.
    PageUp,
    /// `<pgdn>` — half the visible window.
    PageDown,
}

/// Moves the highlight, then resets the preview scroll, because the preview
/// follows the cursor.
fn navigate(_ctx: &mut ActionCtx<'_>, cell: &SkillPickerCell, nav: Nav) {
    cell.update(|picker| {
        let viewport = picker.results_viewport;
        match nav {
            Nav::Up => picker.selection.move_up(viewport),
            Nav::Down => picker.selection.move_down(viewport),
            Nav::PageUp => picker.selection.page_up(viewport),
            Nav::PageDown => picker.selection.page_down(viewport),
        }
        picker.preview_scroll = 0;
    });
}

/// Registers the picker's filter editing hook.
///
/// The composition keymap turns this registration into the scope's
/// printable-character catch-all plus the editing keys, which is why typing in
/// the filter needs no rows of its own.
pub fn register_skill_picker_input_hook(routes: &KeyRoutes, cell: &SkillPickerCell) {
    // The hook outlives this call, so it owns the cell rather than borrowing it.
    let owned = cell.clone();
    let hook: InputHook = Arc::new(move |intent: &EditIntent| {
        owned.update(|picker| match intent {
            EditIntent::InsertChar(ch) => picker.selection.insert_char(*ch),
            EditIntent::DeleteBackward | EditIntent::DeleteForward => {
                picker.selection.backspace();
            }
            EditIntent::CursorLeft => picker.selection.move_cursor_left(),
            EditIntent::CursorRight => picker.selection.move_cursor_right(),
            // The filter is a single-line box: home/end have no meaning here.
            EditIntent::CursorHome | EditIntent::CursorEnd => {}
            EditIntent::Paste(text) => picker.selection.insert_text(text),
        });
        Some(IntentResult::empty())
    });
    routes.register_input_hook(&skill_picker_scope(), hook);
}

/// The session data the picker seeds itself from on open.
struct Seed {
    /// Skills discovered for the active session (cwd-scoped).
    discovered: Vec<Skill>,
    /// Skills currently disabled, snapshotted so ESC can restore them.
    disabled: HashSet<String>,
    /// The active theme, so rows render with the right colors.
    theme: jinn_theme::Theme,
}

// ── Actions ─────────────────────────────────────────────────────────────

/// Opens the picker: seed the rows from the session's discovered skills,
/// snapshot the disabled set for the ESC revert, and push the picker's scope.
fn open_skill_picker(ctx: &mut ActionCtx<'_>, cell: &SkillPickerCell) -> IntentResult {
    let Some(state) = app(ctx) else {
        return IntentResult::empty();
    };
    let session = state.active_session();
    let seed = Seed {
        discovered: session.discovered_skills().to_vec(),
        disabled: session.disabled_skills().clone(),
        theme: state.frontend.theme.clone(),
    };

    cell.update(|picker| {
        skill_picker_actions::open(picker, &seed.discovered, &seed.disabled, &seed.theme);
    });

    IntentResult::empty().with_scope_signal(ScopeSignal::Push(skill_picker_scope()))
}

/// `<enter>`: commit the toggled set as the session's disabled skills and close.
///
/// The snapshot is cleared without restoring — the commit is authoritative.
fn confirm_skill_picker(ctx: &mut ActionCtx<'_>, cell: &SkillPickerCell) -> IntentResult {
    // `update` takes a unit-returning closure, so the committed set is parked
    // beside the cell and read back after the guard is released.
    let mut disabled = HashSet::new();
    cell.update(|picker| disabled = skill_picker_actions::confirm(picker));
    let Some(state) = app(ctx) else {
        return IntentResult::empty();
    };
    state.active_session_mut().set_disabled_skills(disabled);
    IntentResult::empty().with_scope_signal(ScopeSignal::PopIf(skill_picker_scope()))
}

/// `<esc>`: restore the snapshotted disabled set and close.
///
/// The revert path, never the confirm path. Without the pop the user would be
/// stranded inside a picker whose filter no longer reflects the session.
fn cancel_skill_picker(ctx: &mut ActionCtx<'_>, cell: &SkillPickerCell) -> IntentResult {
    let mut restored = None;
    cell.update(|picker| restored = skill_picker_actions::cancel(picker));
    let Some(state) = app(ctx) else {
        return IntentResult::empty();
    };
    if let Some(disabled) = restored {
        state.active_session_mut().set_disabled_skills(disabled);
    }
    IntentResult::empty().with_scope_signal(ScopeSignal::PopIf(skill_picker_scope()))
}

/// `<tab>`: flip the highlighted skill's enabled flag, then advance so a run of
/// adjacent skills can be toggled without moving the cursor first.
///
/// A skill already loaded into context cannot be disabled: the body stays
/// pinned in history until it is unpinned and pruned, so a disabled marker would
/// promise an unload that never happens. For a loaded skill this is a full
/// no-op — the marker and the cursor both stay put.
fn toggle_highlighted_skill(ctx: &mut ActionCtx<'_>, cell: &SkillPickerCell) -> IntentResult {
    let loaded: HashSet<String> =
        app(ctx).map_or_else(HashSet::new, |state| state.active_session().loaded_skills());

    cell.update(|picker| {
        let Some(name) = skill_picker_actions::highlighted_name(picker) else {
            return;
        };
        if loaded.contains(&name) {
            return;
        }
        skill_picker_actions::toggle_highlighted(picker);
        let viewport = picker.results_viewport;
        picker.selection.move_down(viewport);
    });
    IntentResult::empty()
}

/// `<c-l>`: load the highlighted skill into context as a pinned tool
/// call/result pair, leaving the picker open.
///
/// Already-loaded skills get a transient notice instead of a duplicate pair. A
/// disabled skill is auto-enabled *durably*: the name leaves both the
/// cancel-revert snapshot and the session's live set, so neither `<enter>` nor
/// `<esc>` can re-disable a skill the user just loaded.
fn load_highlighted_skill(ctx: &mut ActionCtx<'_>, cell: &SkillPickerCell) -> IntentResult {
    let Some(state) = app(ctx) else {
        return IntentResult::empty();
    };

    // Snapshot what the action needs from the cell; the guard drops here.
    let Some((name, enabled, body)) = highlighted_entry(cell) else {
        return IntentResult::empty();
    };

    // Resolve the file path from the session's discovered set rather than
    // re-deriving it from the global dir — this is what makes project-local
    // skills loadable.
    let Some(skill_path) = state
        .active_session()
        .discovered_skills()
        .iter()
        .find(|s| s.name == name)
        .map(|s| s.file_path.clone())
    else {
        return IntentResult::empty();
    };

    if state.active_session().loaded_skills().contains(&name) {
        state
            .active_session_mut()
            .push_entry(ChatEntry::transient(format!(
                "Skill '{name}' is already loaded"
            )));
        return IntentResult::empty();
    }

    if !enabled {
        enable_durably(state, cell, &name);
    }

    let tool_call_id = ChatEntryId::new().to_string();
    let location = skill_path.to_string_lossy().to_string();
    let xml = format!("<skill name=\"{name}\" location=\"{location}\">\n{body}\n</skill>");
    let arguments = serde_json::json!({ "name": name }).to_string();

    state.active_session_mut().push_entry(ChatEntry::tool_call(
        tool_call_id.clone(),
        "skill",
        arguments,
    ));
    let mut result = ChatEntry::tool_result(tool_call_id, "skill", xml, ToolResultStatus::Success);
    result.pin_position = Some(PinPosition::Relative);
    state.active_session_mut().push_entry(result);

    let session_id = state.active_session().session_id().clone();
    IntentResult::new_message(jinn_session_msg::MarkSessionInteracted { session_id })
}

/// Auto-enables a skill everywhere it could otherwise be re-disabled: its row in
/// the picker, the ESC-revert snapshot, and the session's live set.
fn enable_durably(state: &mut jinn_domain::AppState, cell: &SkillPickerCell, name: &str) {
    cell.update(|picker| {
        if let Some(snapshot) = picker.snapshot.as_mut() {
            snapshot.remove(name);
        }
        if picker
            .selection
            .selected_item()
            .is_some_and(|item| item.entry().name == name)
        {
            picker.selection.with_selected_mut(|item| {
                item.entry_mut().enabled = true;
            });
        }
    });

    let mut disabled = state.active_session().disabled_skills().clone();
    disabled.remove(name);
    state.active_session_mut().set_disabled_skills(disabled);
}

/// `<c-r>`: rescan every discovery source.
///
/// All three scans go out together so discovery settles cleanly, and a
/// transient note explains the pause.
fn refresh_skill_resources(ctx: &mut ActionCtx<'_>, _cell: &SkillPickerCell) -> IntentResult {
    let Some(state) = app(ctx) else {
        return IntentResult::empty();
    };
    state
        .active_session_mut()
        .push_entry(ChatEntry::transient("Refreshing project resources..."));

    let session = state.active_session();
    let session_id = session.session_id().clone();
    let cwd = session.cwd().to_path_buf();

    IntentResult::new_message(jinn_skills_msg::ScanSkills {
        session_id: session_id.clone(),
        cwd: cwd.clone(),
    })
    .with_message(jinn_session_init_msg::RescanPromptTemplates {
        session_id: session_id.clone(),
        cwd: cwd.clone(),
    })
    .with_message(jinn_session_init_msg::ScanContextFiles { session_id, cwd })
}

/// `<c-u>`: scroll the preview pane up one page, saturating at the top.
fn scroll_preview_up(_ctx: &mut ActionCtx<'_>, cell: &SkillPickerCell) -> IntentResult {
    cell.update(|picker| {
        picker.preview_scroll = picker.preview_scroll.saturating_sub(PREVIEW_PAGE_SIZE);
    });
    IntentResult::empty()
}

/// `<c-d>`: scroll the preview pane down one page.
fn scroll_preview_down(_ctx: &mut ActionCtx<'_>, cell: &SkillPickerCell) -> IntentResult {
    cell.update(|picker| {
        picker.preview_scroll = picker.preview_scroll.saturating_add(PREVIEW_PAGE_SIZE);
    });
    IntentResult::empty()
}

/// `<c-c>`: clear the filter while the picker is open, or close it when the
/// filter is already empty — the same clear-or-leave shape the cwd popup uses.
fn clear_filter_or_leave(_ctx: &mut ActionCtx<'_>, cell: &SkillPickerCell) -> IntentResult {
    let had_filter = !cell.read().selection.filter().is_empty();
    cell.update(|picker| picker.selection.clear_filter());
    if had_filter {
        IntentResult::empty()
    } else {
        IntentResult::empty().with_scope_signal(ScopeSignal::PopIf(skill_picker_scope()))
    }
}

/// The highlighted row's name, enabled flag, and body, snapshotted out of the
/// cell so the caller holds no guard.
fn highlighted_entry(cell: &SkillPickerCell) -> Option<(String, bool, String)> {
    let guard = cell.read();
    let item = guard.selection.selected_item()?;
    let entry = item.entry();
    Some((entry.name.clone(), entry.enabled, entry.body.clone()))
}

/// Republishes the picker's rows from a discovery scan.
///
/// Called by the slice's republisher actor when a
/// [`SkillsLoaded`](jinn_skills_msg::SkillsLoaded) event announces a new set.
/// Without it the picker would keep showing the previous scan's rows, and with
/// it the highlight stays on the skill the user was looking at rather than
/// jumping to the top of the list.
///
/// The theme is taken from the picker's own palette, which the open action
/// already captured when the rows were built — repainting must not recolor rows
/// the user is currently looking at.
pub fn republish_from_discovery(cell: &SkillPickerCell, discovered: &[Skill]) {
    cell.update(|picker| {
        let highlight = picker
            .selection
            .selected_item()
            .map(|item| item.entry().name.clone());
        let disabled = picker.snapshot.clone().unwrap_or_default();
        let theme = picker.theme.clone();
        crate::skill_picker_reload::reload_skill_picker(picker, discovered, &disabled, &theme);
        if let Some(name) = highlight {
            restore_highlight(picker, &name);
        }
    });
}

/// Puts the highlight back on `name` after a reload, if it survived the rescan.
fn restore_highlight(picker: &mut SkillPickerState, name: &str) {
    let Some(index) = picker
        .selection
        .items()
        .iter()
        .position(|item| item.entry().name == name)
    else {
        return;
    };
    picker.selection.set_selection(index);
}
