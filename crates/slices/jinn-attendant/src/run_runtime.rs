//! Delivering a message to an attendant as a run.
//!
//! The `tell_session` tool in jinn-tools lands here. This module owns
//! the *composition* of the run — what the dispatched entry says — while
//! the tool owns the *decision* of which delivery path a target takes.
//!
//! Two run preparations exist already, and each is deliberately its own
//! sequence: the manual re-run (`R`) supersedes in-flight work, and the
//! trigger fire queues behind it. An inbound inter-session message is the
//! trigger case in both directions — nobody asked this attendant to throw
//! away what it is doing — so delivery never cancels:
//!
//! - An idle attendant gets a run now, seeded through its template with
//!   the message folded in. Seeding keeps the template's standing
//!   instructions in every dispatch, which is the whole point of a seed.
//! - A busy attendant has the seeded entry queued by the ordinary enqueue
//!   handler; it runs when the in-flight turn finishes.
//!
//! This module never cancels a turn. The manual path's cancel lives in
//! `rerun` and stays there.

use jinn_core_types::chat_entry::ChatEntry;
use jinn_session_state::ChatSessionState;

use crate::activation;

/// Appends inter-session message text to a rendered seed.
///
/// `render_seed_text` produces the template with the prior report and the
/// parent-session header folded in; the message rides beneath all of it,
/// clearly labelled as what this run was asked. When the template renders
/// to nothing (an empty template), the message *is* the prompt — an
/// attendant with no standing instructions still receives its dispatch.
///
/// The message goes in last so a session minting its title from the first
/// line of a dispatched entry still titles itself from the seed, not from
/// whatever one stage said to another.
#[must_use]
pub fn seed_with_message(rendered: Option<String>, message: &str) -> String {
    match rendered {
        Some(seed) => format!("{seed}\n\n---\n\nMESSAGE:\n\n{message}"),
        None => message.to_owned(),
    }
}

/// Builds the run entry for delivering `message` to `attendant`, applying
/// the `Reset` behavior first when the attendant's mode says so.
///
/// Returns the entry to dispatch and the ids whose context the reset
/// changed — non-empty only when a `Reset` actually excluded something,
/// which is the caller's cue to persist the session.
///
/// This is [`activation::prepare_trigger_run`] with the message composed
/// in: same reset, same seed pipeline, same prior-report fold.
#[must_use]
pub fn prepare_message_run(
    session: &mut ChatSessionState,
    message: &str,
) -> (Option<ChatEntry>, Vec<jinn_core_types::ChatEntryId>) {
    let reset = if session.attendant_behavior().resets_context() {
        activation::reset_context(session)
    } else {
        Vec::new()
    };
    let prior = session
        .latest_attendant_report()
        .map(|report| report.body.clone());
    let parent = session.parent_session().as_ref().map_or_else(
        || jinn_attendant_msg::NO_PARENT_SESSION_TEXT.to_owned(),
        ToString::to_string,
    );
    let rendered = activation::render_seed_text(session.seed_template(), prior.as_deref(), &parent);
    let text = seed_with_message(rendered, message);
    (Some(ChatEntry::user_expanded(text.clone(), text)), reset)
}
