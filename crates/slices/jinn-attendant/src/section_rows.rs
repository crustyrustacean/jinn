//! Row building for the Attendants sidebar section.
//!
//! The attendant's *domain* — who its rows are, what their latest report
//! says, whether a report is stale — lives here in the owning slice. The
//! sidebar's section (`jinn-sidebar::sections::attendants_section`) renders
//! these rows but never reconstructs them, which keeps the dependency one
//! way: sidebar → attendant.
//!
//! Staleness: a report is stale when the parent session has produced history
//! since the report was published — that is the parent resuming work. The
//! comparison is per-attendant, so one attendant's fresh report never clears
//! a sibling's stale one.

use jinn_app_state::AppState;
use jinn_attendant_msg::AttendantReport;

/// One attendant row: the session id, its display name, and its latest report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttendantRow {
    /// The attendant session's id.
    pub session_id: jinn_core_types::SessionId,
    /// The session name — the attendant's identity in the UI.
    pub name: String,
    /// The most recent report body, if the attendant has ever reported.
    pub latest_report: Option<String>,
    /// Whether the latest report predates the parent's latest activity.
    pub is_stale: bool,
}

/// Every loaded attendant, sorted by name, with report and staleness data.
#[must_use]
pub fn attendant_rows(state: &AppState) -> Vec<AttendantRow> {
    let mut rows: Vec<AttendantRow> = state
        .session
        .iter()
        .filter(|(_, session)| {
            session.is_attendant()
                && session.session_state() == jinn_session_store_msg::SessionState::Loaded
        })
        .map(|(id, attendant)| {
            let latest = attendant.latest_attendant_report();
            let parent_activity = attendant
                .parent_session()
                .as_ref()
                .and_then(|parent_id| state.session.get(parent_id))
                .map(|parent| *parent.last_history_activity_at());
            let is_stale = match (latest, parent_activity) {
                (Some(report), Some(activity)) => report.published_at < activity,
                _ => false,
            };
            AttendantRow {
                session_id: id.clone(),
                name: attendant.title().unwrap_or("Untitled Session").to_owned(),
                latest_report: latest.map(|report: &AttendantReport| report.body.clone()),
                is_stale,
            }
        })
        .collect();
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    rows
}

/// The highlighted attendant's full report log, for the history picker's
/// opener.
///
/// Returns `None` when nothing is highlighted or the selection is not a
/// loaded attendant; an attendant with no reports yields an empty vec, which
/// the picker renders as its never-reported state.
#[must_use]
pub fn highlighted_reports(state: &AppState) -> Option<Vec<jinn_attendant_msg::AttendantReport>> {
    let index = state.frontend.with_sections(
        |sections: &jinn_sidebar_msg::SidebarSections| sections.attendant.selected_index,
        || None,
    )?;
    let rows = attendant_rows(state);
    let row = rows.get(index)?;
    state
        .session
        .get(&row.session_id)
        .map(|a| a.attendant_reports().to_vec())
}
