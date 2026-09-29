//! Preparing an attendant's context and dispatching its run.

use jinn_attendant_msg::{AttendantActivation, PRIOR_REPORT_PLACEHOLDER};
use jinn_core_types::chat_entry::ChatEntry;
use jinn_core_types::{ChatEntryId, ContextOverride};
use jinn_session_state::ChatSessionState;

/// Builds the seed text for a run, from the seed template and the prior report.
///
/// - The `<prior report>` placeholder is substituted when present.
/// - A template without the placeholder gets the report appended on a new
///   line — the user wrote instructions, the report is additional context.
/// - An empty template means no injection at all.
///
/// Pure; unit-testable without a session.
#[must_use]
pub fn render_seed_text(template: &str, prior_report: Option<&str>) -> Option<String> {
    let Some(prior) = prior_report else {
        return (!template.is_empty()).then(|| template.to_owned());
    };
    if template.is_empty() {
        return None;
    }
    if template.contains(PRIOR_REPORT_PLACEHOLDER) {
        Some(template.replace(PRIOR_REPORT_PLACEHOLDER, prior))
    } else {
        Some(format!("{template}\n\n{prior}"))
    }
}

/// Force-excludes every non-pinned entry in the session's history.
///
/// This is what `Reset` activation means: the model sees only the pins.
/// The human still sees the full transcript; `ForcedExclude` hides an entry
/// from context, not from the UI, and the user can un-hide any entry.
///
/// Pinned entries are skipped explicitly. The history editor's pin-wins
/// guard would refuse the write anyway, but naming the skip here states the
/// intent instead of relying on a refusal to carry it.
///
/// Returns the ids whose context actually changed.
#[must_use]
pub fn reset_context(session: &mut ChatSessionState) -> Vec<jinn_core_types::ChatEntryId> {
    let ids: Vec<jinn_core_types::ChatEntryId> = session
        .history()
        .iter()
        .filter(|entry| !entry.is_pinned())
        .map(|entry| entry.id.clone())
        .collect();
    ids.into_iter()
        .filter_map(|id| {
            session.set_entry_context_override_by_id(&id, ContextOverride::ForcedExclude)
        })
        .collect()
}

/// Resets the session's context if it is in `Reset` mode, then builds the
/// seeded run prompt from the template and the prior report.
///
/// Returns the entry to dispatch, and the ids whose context the reset
/// changed — a caller that persists the session needs to know whether the
/// exclusions are worth writing.
///
/// `Reset` mode means "the model sees only the pins", so the reset is a
/// mutation that has to happen on this path; [`prepare_seed_entry`] stays
/// pure for callers that only need the prompt.
#[must_use]
pub fn prepare_run(session: &mut ChatSessionState) -> (Option<ChatEntry>, Vec<ChatEntryId>) {
    let reset = if session.attendant_activation() == AttendantActivation::Reset {
        reset_context(session)
    } else {
        Vec::new()
    };
    (prepare_seed_entry(session), reset)
}

/// The seed entry a run dispatches, per the session's activation mode.
///
/// Returns `None` in `Continue` mode — the existing conversation carries
/// the run, and no new entry is injected.
#[must_use]
pub fn prepare_seed_entry(session: &ChatSessionState) -> Option<ChatEntry> {
    match session.attendant_activation() {
        AttendantActivation::Continue => None,
        AttendantActivation::Seed | AttendantActivation::Reset => {
            let prior = session
                .latest_attendant_report()
                .map(|report| report.body.clone());
            render_seed_text(session.seed_template(), prior.as_deref())
                .map(|text| ChatEntry::user_expanded(text.clone(), text))
        }
    }
}
