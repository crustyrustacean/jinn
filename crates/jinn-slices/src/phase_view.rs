//! The session phase, as everyone reads it.
//!
//! The one phase representation in the workspace. It is **derived, never
//! authoritative**: the turn ledger (`jinn-kernel`'s `TurnLedger`) owns
//! turn state, and the sessions that carry a phase value hold a display
//! projection the ledger's own admission and settlement paths write. No
//! code decides admission from a `PhaseView` — admission is the ledger's
//! `mint`/`call`/`ack` answers — so this type can never disagree its way
//! into admitting work.

use serde::{Deserialize, Serialize};

/// The phase of one session's turn, as the UI and the guards read it.
///
/// - [`PhaseView::Idle`] — no live turn.
/// - [`PhaseView::Streaming`] — a provider stream is in flight.
/// - [`PhaseView::Working`] — the turn is busy but no stream is in
///   flight: tool calls executing, or the next request being prepared.
/// - [`PhaseView::Cancelling`] — the turn is draining from a cancel;
///   new work is refused until the drain settles.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PhaseView {
    /// No live turn.
    #[default]
    Idle,
    /// A provider stream is in flight.
    Streaming,
    /// Tool calls are executing or the next request is being prepared;
    /// the stream has retired.
    Working,
    /// The turn is draining from a cancel.
    Cancelling,
}

impl PhaseView {
    /// Whether a session in this phase is working.
    ///
    /// The single definition of "working" in the codebase: a session is busy
    /// whenever its phase is not [`PhaseView::Idle`] — the draining phase
    /// included, because the user is still waiting on it. There are
    /// deliberately no carve-outs for a parent orchestrating subagents, a
    /// bot composing a report, or a turn running its tool loop — those
    /// *are* the time the user is waiting.
    #[must_use]
    pub const fn is_working(self) -> bool {
        !matches!(self, Self::Idle)
    }

    /// The lowercase name, for logs and wire surfaces.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Streaming => "streaming",
            Self::Working => "working",
            Self::Cancelling => "cancelling",
        }
    }
}

impl std::fmt::Display for PhaseView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl crate::BusMessage for PhaseView {}

#[cfg(test)]
mod tests {
    use super::PhaseView;

    #[rstest::rstest]
    #[case(PhaseView::Idle, false)]
    #[case(PhaseView::Streaming, true)]
    #[case(PhaseView::Working, true)]
    #[case(PhaseView::Cancelling, true)]
    fn only_idle_is_not_working(#[case] phase: PhaseView, #[case] expected: bool) {
        // Given any phase.
        // When asking whether a session in it is working.
        let working = phase.is_working();
        // Then working is exactly "the phase is not idle".
        assert_eq!(working, expected);
    }

    #[rstest::rstest]
    #[case(PhaseView::Idle, "idle")]
    #[case(PhaseView::Streaming, "streaming")]
    #[case(PhaseView::Working, "working")]
    #[case(PhaseView::Cancelling, "cancelling")]
    fn as_str_names_the_variant(#[case] phase: PhaseView, #[case] expected: &str) {
        // Given any phase.
        // When rendering its name.
        // Then the lowercase name comes back.
        assert_eq!(phase.as_str(), expected);
    }
}
