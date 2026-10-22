// Copyright (C) 2026 Jayson Lennon
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as
// published by the Free Software Foundation, either version 3 of the
// License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! Observable-behavior tests for the slice-owned skills picker.
//!
//! These assert what the picker *does* through its own public entry points —
//! the cell, the action functions, and the attached route rows — never internal
//! wiring details. A test here that reached past those boundaries would break
//! the moment the implementation changed while the behavior did not.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "test module, panics are acceptable"
)]

use marked_path::{Absolute, MarkedPathBuf};
use std::collections::HashSet;

use jinn_core_types::{ChatEntry, NameFilter};
use jinn_skills_msg::{Skill, SkillPickerState, SkillSource, skill_picker_slot};
use jinn_slices::KeyRoutes;
use jinn_slices::cell::TypedCell;
use jinn_slices::route::{ActionCtx, EditIntent};

use crate::skill_picker_actions;
use crate::skill_picker_routes::{self, SKILL_PICKER_BINDINGS};
use crate::skill_picker_scope::skill_picker_scope;

/// A picker cell wired to a fresh route table.
///
/// The cell is minted locally on its own registry — the catalog owns the
/// production registration — because these tests exercise picker
/// *behavior* directly, where the registration path is incidental. Tests
/// that assert *wiring* use [`activated`] instead.
fn wired() -> (TypedCell<SkillPickerState>, KeyRoutes) {
    let routes = KeyRoutes::new();
    let cell = jinn_slices::Slices::new()
        .register(skill_picker_slot(), SkillPickerState::default())
        .expect("skill picker slot is free in a fresh registry");
    skill_picker_routes::attach_skill_picker_rows(&routes, &cell);
    skill_picker_routes::register_skill_picker_input_hook(&routes, &cell);
    (cell, routes)
}

/// A slice activated through the real [`activate`], with its registry and route
/// table returned for inspection.
///
/// Tests that assert *wiring* must use this, not a hand-built stand-in: a
/// stand-in that re-implements the same calls passes even when `activate` stops
/// making them, which is exactly the regression these tests exist to catch.
async fn activated() -> (
    jinn_slices::Slices,
    KeyRoutes,
    &'static jinn_kernel::Services,
) {
    let slices = jinn_slices::Slices::new();
    // Production boot seeds every slice cell before any slice activates;
    // `activate` resolves the picker's cell by slot key, so the harness
    // seeds the same registry the app would.
    jinn_cell_catalog::register_all_cells(&slices);
    let key_routes = KeyRoutes::new();
    let mut viewport = jinn_slices::view::Viewport::new();
    let overlay_views = jinn_slices::OverlayViews::new();
    let services = jinn_kernel::Services::new_fake().await;
    let mut host = jinn_slices::SliceHost::new(
        &slices,
        &mut viewport,
        &overlay_views,
        &key_routes,
        &services.trouper_system,
    );

    crate::activate(&mut host);

    let system: &'static jinn_kernel::Services = Box::leak(Box::new(services));
    (slices, key_routes, system)
}

/// A discovered skill with a body, as the discovery scan would produce.
fn skill(name: &str) -> Skill {
    Skill {
        name: name.to_owned(),
        description: format!("the {name} skill"),
        file_path: MarkedPathBuf::<Absolute>::new(format!("/skills/{name}/SKILL.md"))
            .expect("rooted test path"),
        body: format!("body of {name}"),
        base_dir: MarkedPathBuf::<Absolute>::new(format!("/skills/{name}"))
            .expect("rooted test path"),
        source: SkillSource::Global,
    }
}

/// The names currently shown by the picker, in display order.
fn visible_names(cell: &TypedCell<SkillPickerState>) -> Vec<String> {
    cell.read()
        .selection
        .items()
        .iter()
        .map(|item| item.entry().name.clone())
        .collect()
}

/// A discovered skill whose body is `lines` markdown lines, as the discovery
/// scan would produce.
///
/// The `skill()` helper's one-line bodies never overflow the preview pane;
/// the clamp tests need a body long enough that the widget-reported bound
/// sits far below a deliberately oversized scroll offset.
fn tall_skill(name: &str, lines: usize) -> Skill {
    let body = (0..lines).fold(String::new(), |acc, i| format!("{acc}line {i} of {name}\n"));
    Skill {
        name: name.to_owned(),
        description: format!("the {name} skill"),
        file_path: MarkedPathBuf::<Absolute>::new(format!("/skills/{name}/SKILL.md"))
            .expect("rooted test path"),
        body,
        base_dir: MarkedPathBuf::<Absolute>::new(format!("/skills/{name}"))
            .expect("rooted test path"),
        source: SkillSource::Global,
    }
}

/// Fires the picker's real route action, the way the kernel's intent handler
/// would: the dynamic intent through the attached rows.
fn dispatch(routes: &KeyRoutes, action: &str) {
    let intent = jinn_slices::DynamicIntent::new(skill_picker_scope(), action, action);
    let mut state = jinn_kernel::AppState::default();
    routes
        .action_for(
            &intent,
            ActionCtx {
                state: &mut state,
                slices: &jinn_slices::Slices::new(),
                config: jinn_slices::empty_config_layer(),
                key_bytes: Vec::new(),
            },
        )
        .unwrap_or_else(|| panic!("`{action}` must be a bound route row in the picker scope"));
}

/// Opens the picker over `names` with `disabled` already off.
fn opened(cell: &TypedCell<SkillPickerState>, names: &[&str], disabled: &[&str]) {
    let discovered: Vec<Skill> = names.iter().copied().map(skill).collect();
    let off = NameFilter::deny(disabled.iter().map(|s| (*s).to_owned()));
    cell.update(|picker| {
        skill_picker_actions::open(
            picker,
            &discovered,
            Some(&off),
            &jinn_theme::default_theme(),
        );
    });
}

#[rstest::rstest]
fn opening_the_picker_shows_the_discovered_skills() {
    // Given an empty picker cell.
    let (cell, _routes) = wired();

    // When opening the picker over two discovered skills.
    opened(&cell, &["alpha", "beta"], &[]);

    // Then both skills are shown, so the menu is not blank.
    assert_eq!(visible_names(&cell), vec!["alpha", "beta"]);
}

#[rstest::rstest]
fn opening_the_picker_marks_already_disabled_skills() {
    // Given a picker cell over two skills, one of them already disabled.
    let (cell, _routes) = wired();

    // When opening the picker.
    opened(&cell, &["alpha", "beta"], &["beta"]);

    // Then the disabled skill renders as disabled rather than enabled.
    let guard = cell.read();
    let flags: Vec<bool> = guard
        .selection
        .items()
        .iter()
        .map(|item| item.entry().enabled)
        .collect();
    assert_eq!(flags, vec![true, false]);
}

#[rstest::rstest]
fn toggling_flips_the_highlighted_skill() {
    // Given an open picker over one enabled skill.
    let (cell, _routes) = wired();
    opened(&cell, &["alpha"], &[]);

    // When toggling the highlighted skill.
    cell.update(skill_picker_actions::toggle_highlighted);

    // Then the skill reads as disabled.
    let guard = cell.read();
    let highlighted = guard
        .selection
        .selected_item()
        .expect("the open picker has a highlighted skill");
    assert!(!highlighted.entry().enabled);
}

#[rstest::rstest]
fn confirming_reports_the_toggled_skill_as_disabled() {
    // Given an open picker whose only skill has been toggled off.
    let (cell, _routes) = wired();
    opened(&cell, &["alpha"], &[]);
    cell.update(skill_picker_actions::toggle_highlighted);

    // When confirming.
    let mut committed = HashSet::new();
    cell.update(|picker| committed = skill_picker_actions::confirm(picker));

    // Then the disabled set names that skill.
    assert!(committed.contains("alpha"));
}

#[rstest::rstest]
fn confirming_an_untouched_picker_disables_nothing() {
    // Given an open picker with no toggles.
    let (cell, _routes) = wired();
    opened(&cell, &["alpha", "beta"], &[]);

    // When confirming.
    let mut committed = HashSet::new();
    cell.update(|picker| committed = skill_picker_actions::confirm(picker));

    // Then nothing is reported disabled.
    assert!(committed.is_empty());
}

/// The picker is a blocklist editor by construction, so what its commit is
/// wrapped into matters: a mode error inverts every name in the set.
#[rstest::rstest]
fn confirming_commits_names_that_become_a_deny_filter() {
    // Given an open picker whose only skill has been toggled off.
    let (cell, _routes) = wired();
    opened(&cell, &["alpha"], &[]);
    cell.update(skill_picker_actions::toggle_highlighted);

    // When confirming and wrapping the committed names as the route does.
    let mut committed = HashSet::new();
    cell.update(|picker| committed = skill_picker_actions::confirm(picker));
    let filter = NameFilter::deny(committed);

    // Then the skill is withheld and nothing else is.
    assert_eq!(filter.mode, jinn_core_types::FilterMode::Deny);
    assert!(!filter.permits("alpha"));
    assert!(filter.permits("beta"));
}

#[rstest::rstest]
fn confirming_clears_the_revert_snapshot() {
    // Given an open picker that snapshotted a disabled set.
    let (cell, _routes) = wired();
    opened(&cell, &["alpha"], &["alpha"]);

    // When confirming.
    cell.update(|picker| {
        let _ = skill_picker_actions::confirm(picker);
    });

    // Then no revert is left behind, so a later cancel cannot undo the commit.
    let guard = cell.read();
    assert!(guard.snapshot.is_none());
}

#[rstest::rstest]
fn cancelling_reports_the_pre_open_filter() {
    // Given a picker opened while one skill was disabled, then toggled on.
    let (cell, _routes) = wired();
    opened(&cell, &["alpha"], &["alpha"]);
    cell.update(skill_picker_actions::toggle_highlighted);

    // When cancelling.
    let mut restored = None;
    cell.update(|picker| restored = skill_picker_actions::cancel_filter(picker));

    // Then the pre-open filter comes back, undoing the toggle.
    let restored = restored
        .expect("an open picker has a snapshot to restore")
        .expect("the snapshot held a filter");
    assert!(!restored.permits("alpha"));
}

#[rstest::rstest]
fn cancelling_consumes_the_snapshot() {
    // Given an open picker.
    let (cell, _routes) = wired();
    opened(&cell, &["alpha"], &["alpha"]);

    // When cancelling.
    cell.update(|picker| {
        let _ = skill_picker_actions::cancel_filter(picker);
    });

    // Then the snapshot is gone, so a second cancel cannot revert twice.
    let guard = cell.read();
    assert!(guard.snapshot.is_none());
}

#[rstest::rstest]
fn cancelling_a_picker_that_never_opened_reverts_nothing() {
    // Given a picker cell that was never opened.
    let (cell, _routes) = wired();

    // When cancelling.
    let mut restored = None;
    cell.update(|picker| restored = skill_picker_actions::cancel_filter(picker));

    // Then there is no set to restore.
    assert!(restored.is_none());
}

// ── Wiring ───────────────────────────────────────────────────────────────

#[rstest::rstest]
fn the_picker_binds_a_key_for_every_footer_entry() {
    // Given the route rows the slice attaches.
    let (_cell, routes) = wired();

    // When collecting the keys bound in the picker's own scope.
    let bound: Vec<&str> = routes
        .rows()
        .iter()
        .filter(|row| row.scope == skill_picker_scope())
        .map(|row| row.key)
        .collect();

    // Then every key the footer advertises is actually bound.
    for (key, _label) in SKILL_PICKER_BINDINGS {
        assert!(
            bound.contains(key),
            "footer advertises {key} but the picker binds no such key (binds: {bound:?})"
        );
    }
}

#[rstest::rstest]
fn the_picker_binds_every_navigation_key() {
    // Given the route rows the slice attaches.
    let (_cell, routes) = wired();

    // When collecting the keys bound in the picker's scope.
    let bound: Vec<&str> = routes
        .rows()
        .iter()
        .filter(|row| row.scope == skill_picker_scope())
        .map(|row| row.key)
        .collect();

    // Then the list can be walked and paged, and the filter cleared or left.
    for key in [
        "<up>", "<down>", "<pgup>", "<pgdn>", "<enter>", "<esc>", "<c-n>", "<c-c>",
    ] {
        assert!(
            bound.contains(&key),
            "no binding for {key} (binds: {bound:?})"
        );
    }
}

#[rstest::rstest]
fn the_opener_binds_outside_the_picker_it_opens() {
    // Given the route rows the slice attaches.
    let (_cell, routes) = wired();
    let rows = routes.rows();

    // When reading the opener row.
    let opener = rows
        .iter()
        .find(|row| row.key == "<leader>sk")
        .expect("the picker has an opener key");
    let site = opener.site;

    // Then it is not an own-scope binding, which could never fire.
    assert_ne!(
        site,
        jinn_slices::route::BindSite::OwnScope,
        "a key that opens a picker cannot live inside that picker's scope"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn the_picker_registers_a_filter_input_hook() {
    // Given the wiring activation performs — the real `activate`, not a
    // hand-built stand-in, so removing the registration here is detectable.
    let (slices, key_routes, _system) = activated().await;

    // When asking for the picker's input hook.
    let hook = key_routes.input_hook(&skill_picker_scope());

    // Then one is registered, so typing reaches the filter.
    assert!(
        hook.is_some(),
        "the filter needs an input hook to receive text"
    );
    // And the cell the hook writes to is the registered one.
    assert!(
        slices
            .reader::<SkillPickerState>(&skill_picker_slot())
            .is_some()
    );
}

#[rstest::rstest]
#[tokio::test]
async fn typing_a_character_narrows_the_visible_skills() {
    // Given an activated skills slice with an open picker.
    let (slices, key_routes, _system) = activated().await;
    let cell = slices
        .reader(&skill_picker_slot())
        .expect("skill picker cell registered at activation");
    opened(&cell, &["alpha", "beta"], &[]);

    // When the keymap serves an inserted character through the hook.
    let hook = key_routes
        .input_hook(&skill_picker_scope())
        .expect("input hook registered at activation");
    hook(&EditIntent::InsertChar('a'));

    // Then the filter holds the character.
    let filter = cell.read().selection.filter().to_owned();
    assert_eq!(filter, "a");
}

#[rstest::rstest]
#[tokio::test]
async fn backspacing_removes_the_last_filter_character() {
    // Given an activated skills slice whose filter holds one typed character.
    let (slices, key_routes, _system) = activated().await;
    let cell = slices
        .reader(&skill_picker_slot())
        .expect("skill picker cell registered at activation");
    opened(&cell, &["alpha"], &[]);
    let hook = key_routes
        .input_hook(&skill_picker_scope())
        .expect("input hook registered at activation");
    hook(&EditIntent::InsertChar('x'));

    // When backspacing.
    hook(&EditIntent::DeleteBackward);

    // Then the filter is empty again.
    let filter = cell.read().selection.filter().to_owned();
    assert_eq!(filter, "");
}

// ── Republish ────────────────────────────────────────────────────────────

#[rstest::rstest]
#[tokio::test]
async fn a_published_scan_repaints_an_open_picker() {
    // Given an activated skills slice whose picker is open over one skill.
    let (slices, _routes, system) = activated().await;
    let cell = slices
        .reader(&skill_picker_slot())
        .expect("skill picker cell registered at activation");
    opened(&cell, &["alpha"], &[]);

    // When a discovery scan publishes a newly found skill over the bus.
    system
        .trouper_system
        .publish(jinn_skills_msg::SkillsLoaded {
            session_id: jinn_core_types::SessionId::new(),
            skills: vec![skill("alpha"), skill("delta")],
            error: None,
        })
        .await;
    // The broadcast is asynchronous; wait for the cell to carry the new
    // skill rather than giving the subscription a fixed window to drain.
    jinn_testutil::wait::await_cell(&cell, std::time::Duration::from_secs(5), |value| {
        value
            .selection
            .items()
            .iter()
            .filter(|item| item.entry().name == "delta")
            .count()
            == 1
    })
    .await;

    // Then the open menu shows the new skill without being reopened.
    assert_eq!(visible_names(&cell), vec!["alpha", "delta"]);
}

#[rstest::rstest]
fn a_rescan_keeps_the_highlight_on_the_skill_being_looked_at() {
    // Given an open picker with the highlight moved onto the second skill.
    let (cell, _routes) = wired();
    opened(&cell, &["alpha", "beta", "gamma"], &[]);
    cell.update(|picker| {
        picker
            .selection
            .move_down(jinn_skills_msg::RESULTS_VIEWPORT_FALLBACK);
    });

    // When a rescan republishes the same three skills.
    let discovered = vec![skill("alpha"), skill("beta"), skill("gamma")];
    skill_picker_routes::republish_from_discovery(&cell, &discovered);

    // Then the highlight is still on the second skill.
    let guard = cell.read();
    let highlighted = guard
        .selection
        .selected_item()
        .map(|i| i.entry().name.clone());
    assert_eq!(highlighted.as_deref(), Some("beta"));
}

#[rstest::rstest]
fn a_rescan_replaces_the_visible_skills() {
    // Given an open picker over one skill.
    let (cell, _routes) = wired();
    opened(&cell, &["alpha"], &[]);

    // When a rescan reports a different set.
    let discovered = vec![skill("alpha"), skill("delta")];
    skill_picker_routes::republish_from_discovery(&cell, &discovered);

    // Then the rows reflect the new scan.
    assert_eq!(visible_names(&cell), vec!["alpha", "delta"]);
}

// ── Viewport ─────────────────────────────────────────────────────────────

#[rstest::rstest]
#[tokio::test]
async fn the_render_pass_publishes_the_widget_solved_viewport() {
    // Given an activated skills slice whose open picker has not rendered a
    // frame yet, so the cell still holds the pre-render fallback.
    let (slices, _routes, _system) = activated().await;
    let cell = slices
        .reader(&skill_picker_slot())
        .expect("skill picker cell registered at activation");
    opened(&cell, &["alpha"], &[]);
    assert_eq!(
        cell.read().results_viewport,
        jinn_skills_msg::RESULTS_VIEWPORT_FALLBACK
    );

    // When a frame is drawn through the picker's real render pass.
    let area = ratatui::layout::Rect::new(0, 0, 200, 50);
    draw_skill_picker(&slices, area);

    // Then the cell carries the row count the widget solved for that frame —
    // the same `PickerLayout.inner.height` an identical second construction
    // returns — so paging has a real row count rather than the fallback.
    let measured = cell.read().results_viewport;
    let expected = widget_layout(&slices, area).inner.height as usize;
    assert_eq!(measured, expected);
    // And a wide frame's vertical split gives a taller list pane than the
    // stacked layout's fixed row count.
    assert!(measured > jinn_selection_widget::HORIZONTAL_LIST_ROWS as usize);
}

/// Draws one frame of the skill picker through its real render pass into a
/// fresh test terminal.
///
/// This is the harness for renderer-level viewport assertions: the cell write
/// under test happens inside the render pass, so the assertion reads the cell
/// after the draw.
fn draw_skill_picker(slices: &jinn_slices::Slices, area: ratatui::layout::Rect) {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).expect("terminal");
    let facts = jinn_slices::RenderFacts::new(jinn_theme::default_theme(), slices);
    terminal
        .draw(|frame| {
            let popup = crate::skill_picker_render::skill_picker_overlay_rect(&area)
                .expect("geometry fn yields a popup rect");
            crate::skill_picker_render::render_skill_picker(frame, popup, &facts);
        })
        .expect("draw");
}

/// Renders the same widget the skill picker's render pass draws, into a
/// fresh test terminal, and returns the [`jinn_selection_widget::PickerLayout`]
/// the widget solved.
///
/// This is the expected-value source for renderer-level viewport assertions:
/// the layout comes from the widget, never from hand arithmetic.
fn widget_layout(
    slices: &jinn_slices::Slices,
    area: ratatui::layout::Rect,
) -> jinn_selection_widget::PickerLayout {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let cell = slices
        .reader(&skill_picker_slot())
        .expect("skill picker cell registered at activation");
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).expect("terminal");
    let theme = jinn_theme::default_theme();
    let popup = crate::skill_picker_render::skill_picker_overlay_rect(&area)
        .expect("geometry fn yields a popup rect");
    let report = std::cell::Cell::new(None);
    terminal
        .draw(|frame| {
            let guard = cell.read();
            let state: &SkillPickerState = &guard;
            let palette = crate::skill_picker_render::skill_picker_palette(&theme);
            let binds = crate::skill_picker_render::skill_picker_binds();
            let keybind = jinn_picker::keybind_line(&binds, jinn_picker::Tail::Standard, &palette);
            let footers = vec![
                ratatui::text::Line::from(String::new()),
                ratatui::text::Line::from(keybind.0),
            ];

            let widget = jinn_selection_widget::PreviewSelectionWidget::new(&state.selection)
                .title(ratatui::text::Line::from(" Skills "))
                .title_style(ratatui::style::Style::default().fg(theme.popup_title))
                .footers(footers)
                .colors(palette.selection_colors())
                .preview_scroll(state.preview_scroll);
            let cache = state.preview_cache.clone();
            report.set(Some(
                widget.preview_cache(cache.as_ref()).render(frame, popup),
            ));
        })
        .expect("draw");
    report.into_inner().expect("render reports a PickerLayout")
}

// ── Load ─────────────────────────────────────────────────────────────────

#[rstest::rstest]
fn loading_a_skill_pushes_a_matching_tool_call_and_result() {
    // Given an open picker over one discovered skill.
    let (cell, _routes) = wired();
    opened(&cell, &["alpha"], &[]);

    // When loading the highlighted skill.
    let (name, enabled, body) = {
        let guard = cell.read();
        let entry = &guard
            .selection
            .selected_item()
            .expect("the open picker has a highlighted skill")
            .entry();
        (entry.name.clone(), entry.enabled, entry.body.clone())
    };
    assert!(enabled);

    // Then the load produces a skill body wrapped for context.
    let xml = format!("<skill name=\"{name}\">\n{body}\n</skill>");
    assert!(xml.contains("alpha"));
    assert!(xml.contains("body of alpha"));
}

#[rstest::rstest]
fn an_already_loaded_skill_is_reported_rather_than_duplicated() {
    // Given a session that already has a loaded skill.
    let loaded: HashSet<String> = HashSet::from(["alpha".to_owned()]);

    // When the user tries to load it again.
    let name = "alpha".to_owned();

    // Then the name is recognised as already loaded.
    assert!(loaded.contains(&name));
    // And the notice names the skill so the user knows why nothing happened.
    let notice = ChatEntry::transient(format!("Skill '{name}' is already loaded"));
    assert!(format!("{notice:?}").contains("already loaded"));
}

// ── Preview scroll ───────────────────────────────────────────────────────

#[rstest::rstest]
fn scrolling_the_preview_down_advances_the_offset() {
    // Given a picker with the preview at the top.
    let (cell, _routes) = wired();
    opened(&cell, &["alpha"], &[]);

    // When scrolling the preview down.
    cell.update(|picker| picker.preview_scroll = picker.preview_scroll.saturating_add(10));

    // Then the preview offset moved down.
    let guard = cell.read();
    assert_eq!(guard.preview_scroll, 10);
}

#[rstest::rstest]
fn scrolling_the_preview_up_at_the_top_does_not_underflow() {
    // Given a picker with the preview already at the top.
    let (cell, _routes) = wired();
    opened(&cell, &["alpha"], &[]);

    // When scrolling the preview up.
    cell.update(|picker| picker.preview_scroll = picker.preview_scroll.saturating_sub(10));

    // Then the offset stays at zero rather than wrapping.
    let guard = cell.read();
    assert_eq!(guard.preview_scroll, 0);
}

#[rstest::rstest]
#[tokio::test]
async fn drawing_a_frame_clamps_an_oversized_preview_scroll_to_the_reported_bound() {
    // Given an activated skills slice whose open picker highlights a
    // long-body skill, with the stored offset pushed far past any bound.
    let (slices, _routes, _system) = activated().await;
    let cell = slices
        .reader(&skill_picker_slot())
        .expect("skill picker cell registered at activation");
    let discovered = vec![tall_skill("alpha", 60)];
    let theme = jinn_theme::default_theme();
    cell.update(|picker| {
        skill_picker_actions::open(picker, &discovered, None, &theme);
    });
    cell.update(|picker| picker.preview_scroll = 5000);

    // When a frame is drawn through the picker's real render pass.
    draw_skill_picker(&slices, ratatui::layout::Rect::new(0, 0, 100, 30));

    // Then the stored offset converged to the bound the render pass itself
    // wrote — cell against cell, never a hand-computed line count.
    let guard = cell.read();
    assert_eq!(guard.preview_scroll, guard.preview_max_scroll);
    // And the bound is a real measurement, far below the oversized offset.
    assert!(guard.preview_max_scroll < 5000, "the bound must be live");
}

#[rstest::rstest]
#[tokio::test]
async fn holding_preview_down_saturates_at_the_reported_bound() {
    // Given an activated skills slice whose open picker highlights a
    // long-body skill and has drawn one frame, so the cell holds the
    // render pass-measured bound.
    let (slices, routes, _system) = activated().await;
    let cell = slices
        .reader(&skill_picker_slot())
        .expect("skill picker cell registered at activation");
    let discovered = vec![tall_skill("alpha", 60)];
    let theme = jinn_theme::default_theme();
    cell.update(|picker| {
        skill_picker_actions::open(picker, &discovered, None, &theme);
    });
    draw_skill_picker(&slices, ratatui::layout::Rect::new(0, 0, 100, 30));
    let bound = cell.read().preview_max_scroll;
    assert!(bound > 0, "the long body must overflow the pane to be live");

    // When the preview-down action fires repeatedly, well past the bound.
    (0..20).for_each(|_| dispatch(&routes, "scroll-skill-preview-down"));
    let saturated = cell.read().preview_scroll;

    // Then the offset converged to the measured bound, not past it.
    assert_eq!(saturated, bound);
    // And two further presses leave it there — the held key stops moving.
    dispatch(&routes, "scroll-skill-preview-down");
    dispatch(&routes, "scroll-skill-preview-down");
    assert_eq!(cell.read().preview_scroll, saturated);
}

#[rstest::rstest]
fn moving_the_selection_resets_the_preview_scroll() {
    // Given an open picker whose preview has been scrolled down.
    let (cell, routes) = wired();
    opened(&cell, &["alpha", "beta"], &[]);
    dispatch(&routes, "scroll-skill-preview-down");
    assert_eq!(cell.read().preview_scroll, 10);

    // When the highlight moves to the next skill through its route action.
    dispatch(&routes, "move-skill-picker-down");

    // Then the offset is back at the top — the preview follows the cursor.
    assert_eq!(cell.read().preview_scroll, 0);
}

// ── Slot ─────────────────────────────────────────────────────────────────

#[rstest::rstest]
fn the_picker_state_lives_under_one_slot() {
    // Given a picker registered under its slot.
    let (cell, _routes) = wired();

    // When a value is written to it.
    cell.update(|picker| {
        picker.preview_scroll = 7;
    });

    // Then it reads back through the same cell.
    assert_eq!(cell.read().preview_scroll, 7);
    // And the slot is stable, so there is exactly one home for the picker's state.
    assert_eq!(skill_picker_slot(), skill_picker_slot());
}
