//! Tests for the report-history picker.

#![allow(clippy::expect_used, clippy::panic, reason = "test code")]

use jinn_app_state::AppState;
use jinn_attendant_msg::AttendantReport;
use jinn_kernel::common::state::State;
use jinn_session_state::ChatSessionState;

use crate::report_picker_actions as actions;
use crate::section_rows::{attendant_rows, highlighted_reports};

fn report(run: usize, body: &str) -> AttendantReport {
    AttendantReport {
        run,
        published_at: jiff::Timestamp::from_second(1_000 + run as i64).expect("fixed second"),
        body: body.to_owned(),
    }
}

fn state_with_attendant(reports: Vec<AttendantReport>) -> (State, jinn_core_types::SessionId) {
    let state = State::new(AppState::default_with_scope_focus());
    let id = {
        let mut guard = state.write();
        let parent = ChatSessionState::new();
        let parent_id = parent.session_id().clone();
        let mut attendant = ChatSessionState::new_attendant(&parent, true);
        for r in reports {
            attendant.append_attendant_report(r.body);
        }
        let id = attendant.session_id().clone();
        // The section is scoped to the active session's context, so the
        // attendant must be installed under a parent that is being read.
        guard.session.insert(parent);
        guard.session.insert(attendant);
        guard.session.set_active(parent_id);
        guard
            .frontend
            .update_sections(|s| s.attendant.selected_id = Some(id.clone()));
        id
    };
    (state, id)
}

#[rstest::rstest]
#[test]
fn picker_rows_are_newest_first() {
    // Given three reports published in run order.
    let mut picker = jinn_attendant_msg::AttendantReportPickerState::default();

    // When the picker opens over them.
    actions::open(
        &mut picker,
        vec![report(1, "first"), report(2, "second"), report(3, "third")],
    );

    // Then the first row is the newest report.
    let first = actions::highlighted(&picker).expect("a highlighted report");
    assert_eq!(first.run, 3);
    assert_eq!(picker.selection.filtered_count(), 3);
}

#[rstest::rstest]
#[test]
fn picker_on_an_attendant_that_never_reported_is_empty() {
    // Given an attendant with no reports.
    let mut picker = jinn_attendant_msg::AttendantReportPickerState::default();

    // When the picker opens.
    actions::open(&mut picker, Vec::new());

    // Then there is nothing to highlight and the count is zero — the render
    // pass turns this into the never-reported marker.
    assert!(actions::highlighted(&picker).is_none());
    assert_eq!(picker.selection.filtered_count(), 0);
}

#[rstest::rstest]
#[test]
fn opener_snapshots_the_highlighted_attendants_reports() {
    // Given an attendant with two reports, highlighted in the section.
    let (state, _id) = state_with_attendant(vec![report(1, "one"), report(2, "two")]);

    // When the opener resolves the reports to browse.
    let reports = highlighted_reports(&state.read()).expect("the highlighted attendant");

    // Then the snapshot holds both, in storage order (the picker reverses).
    assert_eq!(reports.len(), 2);
    assert_eq!(reports.first().expect("first report").body, "one");
}

#[rstest::rstest]
#[test]
fn opener_returns_none_when_the_section_has_no_cursor() {
    // Given an attendant with a report but no section selection.
    let state = State::new(AppState::default());
    {
        let mut guard = state.write();
        let mut attendant = ChatSessionState::new_attendant(&ChatSessionState::new(), true);
        attendant.append_attendant_report("a finding".to_owned());
        guard.session.insert(attendant);
    }

    // When the opener resolves the reports to browse.
    let reports = highlighted_reports(&state.read());

    // Then there is nothing highlighted to browse.
    assert!(reports.is_none());
}

#[rstest::rstest]
#[test]
fn section_rows_exclude_non_attendant_sessions() {
    // Given a parent with one attendant alongside it, viewing the parent.
    let state = State::new(AppState::default());
    {
        let mut guard = state.write();
        let parent = ChatSessionState::new();
        let parent_id = parent.session_id().clone();
        let attendant = ChatSessionState::new_attendant(&parent, true);
        guard.session.insert(parent);
        guard.session.insert(attendant);
        guard.session.set_active(parent_id);
    }

    // When the section's rows are built.
    let rows = attendant_rows(&state.read());

    // Then only the attendant appears.
    assert_eq!(rows.len(), 1);
}

#[rstest::rstest]
#[test]
fn viewport_measurement_never_windows_against_zero() {
    // Given a popup too small to hold its chrome.
    let popup = ratatui::layout::Rect::new(0, 0, 4, 3);

    // When the visible row count is measured.
    let viewport = crate::report_picker_viewport::results_viewport(popup);

    // Then at least one row is visible, so paging keeps working.
    assert!(viewport >= 1);
}
