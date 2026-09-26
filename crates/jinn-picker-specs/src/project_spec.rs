//! The project picker's spec — behavior authored once in the builder.
//!
//! Lists curated project directories and turns a selection into a new
//! session: plain confirm starts the blank lifecycle at the chosen dir,
//! `<c-enter>` chains into the session-lifecycle picker, and `<c-d>`
//! removes the highlighted project (the picker stays open). Enter carries
//! no close signal — the lifecycle setup owns the scope transition. The
//! `<c-n>` add-directory opener belongs to the preferences slice (bound in
//! this picker's static scope).

use jinn_picker::ActionCtx;
use jinn_picker::PickerEntry;
use jinn_picker::PickerId;
use jinn_picker::PickerOutcome;
use jinn_picker::PickerSpec;
use jinn_picker::RowCtx;
use ratatui::text::Line;

use jinn_domain::PickerKind;
use jinn_domain::common::app_state::AppState;
use jinn_domain::feat::ui::frontend_state::PendingSessionCreation;
use jinn_domain::feat::ui::picker_states::PickerExt;
use jinn_preferences_config::ConfigLayer;
use jinn_preferences_config::schemas::ProjectConfig;
use jinn_project_msg::ProjectEntry;
use jinn_project_msg::render_project_row;
use jinn_slices::FocusScope;

/// Renders one project row — the same tilde-compressed line trunk drew via
/// `ProjectEntry: PickerItem`, now routed through the spec.
fn project_row(entry: &ProjectEntry, ctx: &RowCtx<'_>) -> Line<'static> {
    render_project_row(
        &entry.display,
        ctx.is_selected,
        ctx.match_ranges,
        &entry.theme,
    )
}

/// Downcasts the host's `Any` state to `AppState`.
#[expect(
    clippy::expect_used,
    reason = "domain host lends AppState; a wrong downcast is a wiring bug"
)]
fn state_of<'a>(ctx: &'a mut ActionCtx<'_>) -> &'a mut AppState
where
{
    ctx.state_any()
        .downcast_mut::<AppState>()
        .expect("domain host lends AppState")
}

/// Loads project entries into the picker: one row per curated directory,
/// display strings precomputed (tilde-compressed) and sorted by display.
pub fn load_project_entries(
    frontend: &mut jinn_domain::feat::ui::frontend_state::FrontendState,
    config: &ConfigLayer,
) {
    let projects = config.get_list::<ProjectConfig>().unwrap_or_default();
    set_project_entries(frontend, &projects);
}

/// Rebuilds the project picker's items from an already-read project list.
///
/// Split from [`load_project_entries`] because a spec action cannot hold
/// the state lend and the configuration read at the same time: it reads
/// the list, ends the lend, then rebuilds.
fn set_project_entries(
    frontend: &mut jinn_domain::feat::ui::frontend_state::FrontendState,
    projects: &[ProjectConfig],
) {
    let theme = frontend.theme.clone();
    let entries: Vec<ProjectEntry> = jinn_project_msg::project_entries(projects, &theme);
    let wrapped = crate::build_picker_registry()
        .make_items(jinn_picker::PROJECT_ID, entries)
        .unwrap_or_default();
    frontend.project_picker_mut().set_items(wrapped);
}

/// The path of the project the picker currently highlights.
fn path_of_selected(ctx: &mut ActionCtx<'_>) -> std::path::PathBuf {
    ctx.state_any()
        .downcast_mut::<AppState>()
        .and_then(|state| {
            state
                .frontend
                .project_picker()
                .selected_item()
                .map(|entry| entry.entry().path.clone())
        })
        .unwrap_or_default()
}

/// Builds the project picker's spec.
#[must_use]
pub fn project_spec() -> PickerSpec<ProjectEntry> {
    PickerSpec::new(PickerId::new(jinn_picker::PROJECT_ID))
        .title(" Projects ")
        .row(project_row)
        .search(|entry| entry.display.clone())
        .on_open(|ctx| {
            // Fresh filter + selection each open; entries come straight from
            // the curated preferences (synchronous — no actor round-trip).
            if let Some(picker) =
                ctx.selection::<jinn_selection_widget::SelectionState<PickerEntry<ProjectEntry>>>()
            {
                picker.reset();
            }
            // The state lend and the config read cannot overlap, so the
            // entries are rebuilt in two steps: read the projects out of
            // the layer, then hand them to the picker.
            let projects = ctx
                .config()
                .clone()
                .get_list::<ProjectConfig>()
                .unwrap_or_default();
            let state = state_of(ctx);
            set_project_entries(&mut state.frontend, &projects);
            PickerOutcome::empty()
        })
        .bind("<c-enter>", "new+lifecycle", |ctx| {
            // Stash the chosen dir and pop the picker, then chain into the
            // session-lifecycle picker via the REAL registry — its open hook
            // (not a legacy loader) now fills the entries.
            let (path, starting_cwd) = {
                let state = state_of(ctx);
                let Some(entry) = state.frontend.project_picker().selected_item() else {
                    return PickerOutcome::empty();
                };
                let path = entry.entry().path.clone();
                (path.clone(), path)
            };
            let state = state_of(ctx);
            state.frontend.pending_creation = Some(PendingSessionCreation {
                project_dir: path,
                starting_cwd,
            });
            state.frontend.scope_pop();
            state.frontend.scope_push(FocusScope::Picker {
                kind: PickerKind::SessionLifecycle,
            });
            let registry = crate::build_picker_registry();
            let result = jinn_domain::feat::picker::action::run_active_hook(
                state,
                &registry,
                jinn_domain::feat::picker::action::Hook::Open,
                crate::empty_config_layer(),
            );
            PickerOutcome::from_route_result(result)
        })
        .bind("<c-d>", "remove", |ctx| {
            // Remove the highlighted project from the curated list and
            // refresh in place — the picker stays open.
            // Cloning the handle detaches the read from `ctx`, so the
            // state lend below is not fighting an outstanding borrow.
            let config = ctx.config().clone();
            let path = path_of_selected(ctx);
            remove_project(&config, &path);
            let projects = config.get_list::<ProjectConfig>().unwrap_or_default();
            let state = state_of(ctx);
            set_project_entries(&mut state.frontend, &projects);
            PickerOutcome::empty()
        })
        .on_confirm(|ctx| {
            // Stash the chosen dir, pop, and run the blank lifecycle setup.
            // The setup owns the scope transition (it lands the new session
            // in Normal scope), so this outcome carries no close signal.
            let state = state_of(ctx);
            let Some(entry) = state.frontend.project_picker().selected_item() else {
                return PickerOutcome::empty();
            };
            let path = entry.entry().path.clone();
            state.frontend.pending_creation = Some(PendingSessionCreation {
                project_dir: path.clone(),
                starting_cwd: path,
            });
            state.frontend.scope_pop();
            let result =
                jinn_domain::feat::session_lifecycle::intent::handle_session_lifecycle_setup(
                    state,
                    "",
                    &[],
                    None,
                    jinn_domain::common::render_ctx::empty_config_layer(),
                );
            PickerOutcome::from_route_result(result)
        })
}
/// Removes a project from the `[[project.entry]]` section.
///
/// A failed write is silently dropped: the entry has already left the
/// picker, and surfacing an error there would strand the user in a list
/// that no longer matches the file. A later write repairs the file.
pub(crate) fn remove_project(config: &ConfigLayer, path: &std::path::Path) {
    let remaining = config
        .get_list::<ProjectConfig>()
        .unwrap_or_default()
        .into_iter()
        .filter(|project| project.path != path)
        .collect::<Vec<_>>();
    drop(config.put_list::<ProjectConfig>(&remaining));
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test module, panics are acceptable"
    )]
    use super::*;
    use crate::build_picker_registry;
    use jinn_domain::PickerKind;
    use jinn_domain::common::app_state::AppState;
    use jinn_domain::feat::ui::picker_states::PickerExt;
    use jinn_picker::PROJECT_ID;
    use jinn_preferences_config::schemas::ProjectConfig;
    use jinn_session_state::ChatSessionState;
    use jinn_slices::FocusScope;

    /// A configuration layer whose `[[project.entry]]` section holds
    /// exactly `paths` — the curated list the picker reads and writes.
    fn config_with_projects(paths: &[&str]) -> ConfigLayer {
        use std::fmt::Write as _;
        let mut document = String::new();
        for path in paths {
            writeln!(document, "[[project.entry]]\npath = \"{path}\"").expect("write to String");
        }
        jinn_config::testutil::config_layer(&document)
    }

    /// State with an active origin session (cwd distinct from the project
    /// dirs), the project picker open, and its entries loaded from `config`
    /// — the curated list lives in the document, not in the state.
    fn state_with_projects(config: &ConfigLayer) -> AppState {
        let mut state = AppState::default_with_scope_focus();
        let origin = ChatSessionState::new();
        state.session.insert(origin);
        state
            .session
            .set_active(state.session.active_session_id().clone());
        state
            .active_session_mut()
            .set_cwd(std::path::PathBuf::from("/tmp/active-session-cwd"));
        state.frontend.scope_push(FocusScope::Picker {
            kind: PickerKind::Project,
        });
        load_project_entries(&mut state.frontend, config);
        // index 0 is selected by default after set_items + reset.
        state
    }

    #[rstest::rstest]
    fn open_loads_curated_entries_with_tilde_displays() {
        // Given an app with two curated projects and the project picker open.
        let config = config_with_projects(&["/tmp/project-a", "/tmp/project-b"]);
        let mut state = state_with_projects(&config);
        let registry = build_picker_registry();

        // When opening the picker through the real open path.
        let result = jinn_domain::feat::picker::intent::handle_open_picker(
            &mut state,
            PickerKind::Project,
            &registry,
            &config,
        );

        // Then the open hook ran clean (synchronous load, no messages).
        assert!(result.message_names.is_empty());
        // And the picker holds both curated dirs, display-sorted.
        let items = state.frontend.project_picker().items().to_vec();
        assert_eq!(items.len(), 2);
        let mut displays: Vec<&str> = items.iter().map(|i| i.entry().display.as_str()).collect();
        displays.sort_unstable();
        assert_eq!(displays, vec!["/tmp/project-a", "/tmp/project-b"]);
    }

    #[rstest::rstest]
    fn confirm_creates_new_session_at_chosen_dir() {
        // Given a project picker whose highlighted entry is /tmp/project-a.
        let config = config_with_projects(&["/tmp/project-a", "/tmp/project-b"]);
        let mut state = state_with_projects(&config);
        let registry = build_picker_registry();

        // When confirming the highlighted project (Enter).
        let result = jinn_domain::feat::picker::intent::handle_picker_confirm(
            &mut state, &registry, &config,
        );

        // Then a new session was created (a message was emitted to drive it).
        assert!(!result.0.message_names.is_empty());
        // And the new active session's CWD is the chosen project dir, not the
        // previously active session's CWD.
        assert_eq!(
            state.active_session().cwd(),
            std::path::Path::new("/tmp/project-a"),
        );
        // And the stash was consumed.
        assert!(state.frontend.pending_creation.is_none());
    }

    #[rstest::rstest]
    fn confirm_leaves_previous_session_cwd_unchanged() {
        // Given a project picker with an existing active session.
        let config = config_with_projects(&["/tmp/project-a"]);
        let mut state = state_with_projects(&config);
        let prev_id = state.session.active_session_id().clone();
        let registry = build_picker_registry();

        // When confirming the highlighted project.
        let _result = jinn_domain::feat::picker::intent::handle_picker_confirm(
            &mut state, &registry, &config,
        );

        // Then the previous session (now backgrounded) keeps its original CWD.
        let prev = state
            .session
            .get(&prev_id)
            .expect("previous session still exists");
        assert_eq!(prev.cwd(), std::path::Path::new("/tmp/active-session-cwd"));
    }

    #[rstest::rstest]
    fn ctrl_enter_chains_into_lifecycle_picker_with_entries() {
        // Given a project picker whose highlighted entry is /tmp/project-a.
        let config = config_with_projects(&["/tmp/project-a"]);
        let mut state = state_with_projects(&config);
        let _registry = build_picker_registry();

        // When pressing <c-enter> (new + lifecycle).
        let _result = jinn_domain::feat::picker::action::run_action(
            &mut state,
            &build_picker_registry(),
            PROJECT_ID,
            "<c-enter>",
            &config,
        );

        // Then the project scope was popped and the lifecycle picker opened.
        assert!(matches!(
            state.frontend.scope(),
            FocusScope::Picker {
                kind: PickerKind::SessionLifecycle
            }
        ));
        // And the lifecycle picker holds entries (the real registry ran the
        // lifecycle spec's open hook — the empty-picker regression is fixed).
        assert!(
            !state.frontend.session_lifecycle_picker().items().is_empty(),
            "lifecycle picker must be populated by the chained open"
        );
        // And the chosen dir is stashed in a pending creation, awaiting the
        // lifecycle/args confirm chain.
        let pending = state
            .frontend
            .pending_creation
            .as_ref()
            .expect("pending creation stashed");
        assert_eq!(pending.project_dir, std::path::Path::new("/tmp/project-a"));
        assert_eq!(pending.starting_cwd, std::path::Path::new("/tmp/project-a"));
    }

    #[rstest::rstest]
    fn ctrl_d_removes_highlighted_and_stays_open() {
        // Given a project picker with two entries and the first highlighted.
        let config = config_with_projects(&["/tmp/project-a", "/tmp/project-b"]);
        let mut state = state_with_projects(&config);

        // When removing the highlighted entry (<c-d>).
        let result = jinn_domain::feat::picker::action::run_action(
            &mut state,
            &build_picker_registry(),
            PROJECT_ID,
            "<c-d>",
            &config,
        );

        // Then the highlighted entry is removed from the section.
        let paths: Vec<_> = config
            .get_list::<ProjectConfig>()
            .expect("section reads")
            .iter()
            .map(|p| p.path.clone())
            .collect();
        assert_eq!(paths, vec![std::path::PathBuf::from("/tmp/project-b")]);
        // And no message is published — the write is the effect.
        assert!(result.message_names.is_empty());
        // And the picker stayed open, now showing one entry.
        assert!(matches!(
            state.frontend.scope(),
            FocusScope::Picker {
                kind: PickerKind::Project
            }
        ));
        assert_eq!(state.frontend.project_picker().items().len(), 1);
    }

    #[rstest::rstest]
    fn ctrl_d_on_an_empty_picker_is_a_noop() {
        // Given a project picker with no curated projects.
        let config = config_with_projects(&[]);
        let mut state = state_with_projects(&config);

        // When removing the highlighted entry (<c-d>) — there is none.
        let result = jinn_domain::feat::picker::action::run_action(
            &mut state,
            &build_picker_registry(),
            PROJECT_ID,
            "<c-d>",
            &config,
        );

        // Then nothing happened: no messages, and the curated list is untouched.
        assert!(result.message_names.is_empty());
        assert!(
            config
                .get_list::<ProjectConfig>()
                .expect("section reads")
                .is_empty()
        );
    }
}
