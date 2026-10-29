//! Delivering a message to a bot as a run.
//!
//! The `tell_session` tool in jinn-tools lands here. This module owns
//! the *composition* of the run — what the dispatched entry says — while
//! the tool owns the *decision* of which delivery path a target takes.
//!
//! Two run preparations exist already, and each is deliberately its own
//! sequence: the manual re-run (`R`) supersedes in-flight work, and the
//! trigger fire queues behind it. An inbound inter-session message is the
//! trigger case in both directions — nobody asked this bot to throw
//! away what it is doing — so delivery never cancels:
//!
//! - An idle bot gets a run now, seeded through its template with
//!   the message folded in. Seeding keeps the template's standing
//!   instructions in every dispatch, which is the whole point of a seed.
//! - A busy bot has the seeded entry queued by the ordinary enqueue
//!   handler; it runs when the in-flight turn finishes.
//!
//! The dispatched entry is a [`ChatEntryKind::Intersession`] — source the
//! parent (the bot itself when headless), target the bot — so a
//! dispatch renders like every other session→session message. The text is
//! the composed seed, never wrapped in an attribution prefix.
//!
//! This module never cancels a turn. The manual path's cancel lives in
//! `rerun` and stays there.

use jinn_core_types::chat_entry::ChatEntry;
use jinn_session_state::ChatSessionState;

use crate::activation;

/// Appends inter-session message text to a rendered seed.
///
/// `render_seed_text` produces the template with its tokens substituted;
/// the message rides beneath the seed, clearly labelled as what this run
/// was asked. When the seed renders to nothing (an empty template), the
/// message *is* the prompt — no separator envelope, because there are no
/// standing instructions for it to sit beneath.
///
/// The message goes in last so the bot's title — minted at dispatch
/// from the seed's first non-empty line — describes the standing
/// instructions, not whatever one stage said to another. An empty seed
/// leaves the message first, so the title mints from the message.
#[must_use]
pub fn seed_with_message(rendered: Option<String>, message: &str) -> String {
    match rendered {
        Some(seed) if !seed.trim().is_empty() => {
            format!("{seed}\n\n---\n\nMESSAGE:\n\n{message}")
        }
        _ => message.to_owned(),
    }
}

/// Builds the run entry for delivering `message` to `bot`, applying
/// the `Reset` behavior first when the bot's mode says so.
///
/// Returns the entry to dispatch and the ids whose context the reset
/// changed — non-empty only when a `Reset` actually excluded something,
/// which is the caller's cue to persist the session.
///
/// This is [`activation::prepare_trigger_run`] with the message composed
/// in: same reset, same seed pipeline, same prior-report fold, same
/// Intersession entry kind — [`activation::dispatch_entry`] is the shared
/// construction site, so kind, source fallback, and title mint cannot
/// drift between the three paths.
#[must_use]
pub fn prepare_message_run(
    session: &mut ChatSessionState,
    message: &str,
) -> (Option<ChatEntry>, Vec<jinn_core_types::ChatEntryId>) {
    let reset = if session.bot_behavior().resets_context() {
        activation::reset_context(session)
    } else {
        Vec::new()
    };
    let prior = session
        .latest_bot_report()
        .map(|report| report.body.clone());
    let parent = session.parent_session().as_ref().map_or_else(
        || jinn_bot_msg::NO_PARENT_SESSION_TEXT.to_owned(),
        ToString::to_string,
    );
    let rendered = activation::render_seed_text(session.seed_template(), prior.as_deref(), &parent);
    let text = seed_with_message(Some(rendered), message);
    (Some(activation::dispatch_entry(session, &text)), reset)
}
