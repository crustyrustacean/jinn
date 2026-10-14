//! Preparing an attendant's context and dispatching its run.
//!
//! Every dispatch is a [`ChatEntryKind::Intersession`](jinn_core_types::chat_entry::ChatEntryKind::Intersession) entry — source is the
//! parent (the attendant itself when headless), target the attendant — so a
//! run renders in the subagent color the same way every other session→session
//! message does. The text is the composed seed, never wrapped in an
//! attribution prefix: the seed's own `---`/`MESSAGE:` structure is the
//! formatting.

use jinn_attendant_msg::{
    NO_PARENT_SESSION_TEXT, NO_PRIOR_REPORT_TEXT, PARENT_SESSION_HEADER, PRIOR_REPORT_PLACEHOLDER,
};
use jinn_core_types::chat_entry::ChatEntry;
use jinn_core_types::{ChatEntryId, ContextOverride};
use jinn_session_state::ChatSessionState;

/// Builds the seed text for a run, from the seed template and the prior report.
///
/// - The `<prior report>` placeholder is substituted when present.
/// - A template without the placeholder gets **nothing** from the prior
///   report: the token is the only inlet. A report without a token is
///   dropped, not appended — silently applying a conclusion to work it
///   was never about is out-of-order context, and a template that does
///   not ask for history must not receive any.
/// - With no prior report the placeholder becomes [`NO_PRIOR_REPORT_TEXT`],
///   so no run ever dispatches a raw template token.
/// - `parent_session_id` is appended to every prompt, after the user's own
///   text. An attendant runs in its own session and cannot otherwise reach
///   the transcript it reports on; a session-search tool needs an id to
///   search by.
/// - An empty template means no injection at all.
///
/// Pure; unit-testable without a session.
#[must_use]
pub fn render_seed_text(
    template: &str,
    prior_report: Option<&str>,
    parent_session_id: &str,
) -> Option<String> {
    if template.is_empty() {
        return None;
    }
    let body = match prior_report {
        // The template asked for the report and there is one.
        Some(prior) if template.contains(PRIOR_REPORT_PLACEHOLDER) => {
            template.replace(PRIOR_REPORT_PLACEHOLDER, prior)
        }
        // The template asked and there is nothing to fold in: a first run
        // says so, rather than handing the model a bare template token.
        None if template.contains(PRIOR_REPORT_PLACEHOLDER) => {
            template.replace(PRIOR_REPORT_PLACEHOLDER, NO_PRIOR_REPORT_TEXT)
        }
        // No placeholder: the report never enters the prompt, whichever
        // way `prior_report` came out. The token is the only inlet.
        _ => template.to_owned(),
    };
    Some(format!(
        "{body}\n\n{PARENT_SESSION_HEADER} {parent_session_id}"
    ))
}

/// Force-excludes every non-pinned entry in the session's history.
///
/// This is what the `reset` behavior means: the model sees only the pins.
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
/// seeded run prompt for a *manual* re-run.
///
/// Returns the entry to dispatch, and the ids whose context the reset
/// changed — a caller that persists the session needs to know whether the
/// exclusions are worth writing.
///
/// Seeding is unconditional here: a manual re-run is the user asking the
/// question again, so it always goes through the template. `Preserve` mode
/// governs what an unattended trigger fire does, not this.
#[must_use]
pub fn prepare_manual_run(session: &mut ChatSessionState) -> (Option<ChatEntry>, Vec<ChatEntryId>) {
    let reset = if session.attendant_behavior().resets_context() {
        reset_context(session)
    } else {
        Vec::new()
    };
    (seed_entry(session), reset)
}

/// Resets the session's context if it resets, then builds the seeded run
/// prompt for a *trigger* fire.
///
/// The behavior governs *context*, never *dispatch*. Every behavior
/// dispatches a message; what differs is what the run sees. `Preserve` runs
/// on the context as it stands, and still dispatches the template — the
/// standing instructions that describe what this attendant is for. It folds
/// the prior report in like every other behavior: the report is the
/// attendant's own record of what it found last time, and preserving context
/// means not forgetting it. The difference from `Reset` is what happens to
/// the *history*, not what the prompt says.
///
/// An attendant still being composed dispatches nothing at all. The gate
/// lives here rather than only at the call site so that a trigger fire and a
/// manual re-run cannot disagree about what a preparing attendant does, and
/// it is a gate rather than a caller-side check because `None` for a seed
/// entry is the same shape as an empty template — a run with nothing to say
/// is legitimately a no-dispatch.
///
/// The earlier design returned `None` here for `Preserve`, on the reasoning
/// that an unattended fire "never injects a message the user did not ask
/// for". That is what a template *is*: the user wrote it, and it is the only
/// statement of the attendant's purpose. Suppressing it left a
/// `ParentCompleted` + `Preserve` attendant permanently inert with nothing
/// on screen saying why.
#[must_use]
pub fn prepare_trigger_run(
    session: &mut ChatSessionState,
) -> (Option<ChatEntry>, Vec<ChatEntryId>) {
    if session.attendant_is_prepping() {
        return (None, Vec::new());
    }
    let reset = if session.attendant_behavior().resets_context() {
        reset_context(session)
    } else {
        Vec::new()
    };
    (seed_entry(session), reset)
}

/// The prompt a run dispatches: the template with the prior report folded in.
///
/// The entry is [`ChatEntryKind::Intersession`](jinn_core_types::chat_entry::ChatEntryKind::Intersession) — source the parent session
/// (the attendant itself when it has none), target the attendant, `source_name`
/// always `None`: a dispatch is not an attributed message, the seed's own
/// structure is the formatting. Building the entry is also the title mint
/// point: an attendant never sends a genuine user message, so the queue actor's
/// user-keyed mint would leave it untitled forever. See [`dispatch_entry`].
#[must_use]
fn seed_entry(session: &mut ChatSessionState) -> Option<ChatEntry> {
    let prior = session
        .latest_attendant_report()
        .map(|report| report.body.clone());
    let parent = session
        .parent_session()
        .as_ref()
        .map_or_else(|| NO_PARENT_SESSION_TEXT.to_owned(), ToString::to_string);
    render_seed_text(session.seed_template(), prior.as_deref(), &parent)
        .map(|text| dispatch_entry(session, &text))
}

/// Builds the Intersession dispatch entry for `text`, minting the title.
///
/// One construction site shared by every dispatch path — trigger fire,
/// manual re-run, and `tell_session` delivery — so the entry kind, the
/// source fallback, and the title mint cannot drift between them.
#[must_use]
pub(crate) fn dispatch_entry(session: &mut ChatSessionState, text: &str) -> ChatEntry {
    mint_title_from_seed(session, text);
    let source = session
        .parent_session()
        .clone()
        .unwrap_or_else(|| session.session_id().clone());
    ChatEntry::intersession(source, None, session.session_id().clone(), text.to_owned())
}

/// Titles an untitled attendant from the seed's first non-empty line.
///
/// The first line of the composed text is the seed's standing instructions —
/// [`crate::run_runtime::seed_with_message`] deliberately puts the message
/// last so a title minted here describes the attendant, not one dispatch.
/// Silent on a titled attendant, and never writes an empty string: a template
/// opening with blank lines would otherwise flip the guard to `Some("")` and
/// permanently block every later mint.
fn mint_title_from_seed(session: &mut ChatSessionState, text: &str) {
    if session.title().is_some() {
        return;
    }
    let title = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_owned);
    if let Some(title) = title {
        session.set_title(title);
    }
}
