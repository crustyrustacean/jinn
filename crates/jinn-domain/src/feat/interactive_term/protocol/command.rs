//! Request messages the coordinator actor answers.

use serde::{Deserialize, Serialize};

use crate::feat::interactive_term::pty_session::ExitInfo;

/// Unique identifier of an interactive-term session.
///
/// Short and human-friendly (the model passes it back in
/// `interactive_term_send`/`kill` calls); unique per app run.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
pub struct TermSessionId(pub String);

impl TermSessionId {
    /// Generates a fresh session id (`term-<n>`, monotonically increasing).
    #[must_use]
    pub fn next() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        Self(format!("term-{}", COUNTER.fetch_add(1, Ordering::Relaxed)))
    }
}

impl std::fmt::Display for TermSessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Who may send input to the session right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ControlHolder {
    /// The agent (tool calls) may send input.
    #[default]
    Agent,
    /// The user took over from the terminal tab; agent input is refused.
    User,
}

/// Outcome of a settle wait for a spawn or send.
#[derive(Debug, Clone)]
pub struct TermScreen {
    /// The rendered screen (plain text, trailing blank rows trimmed).
    pub screen: String,
    /// Set when the process exited during (or before) this call.
    pub exited: Option<ExitInfo>,
}

/// Spawn a new interactive session running `command`.
#[derive(Debug, Clone)]
pub struct SpawnTerm {
    /// Shell command to run (passed to `bash -c`).
    pub command: String,
    /// Working directory for the child.
    pub cwd: std::path::PathBuf,
    /// Requested pty size (rows, cols).
    pub size: (u16, u16),
    /// How long to wait for output to settle before replying.
    pub max_wait: std::time::Duration,
}

/// Outcome of a [`SpawnTerm`] request.
#[derive(Debug, Clone, kameo::Reply)]
pub enum SpawnTermOutcome {
    /// Session created; screen captured after the initial settle.
    Started {
        /// The new session's id.
        session_id: TermSessionId,
        /// The post-settle screen.
        screen: TermScreen,
    },
    /// The command failed to spawn (e.g. binary not found).
    Failed(String),
}

/// Send input to a session and wait for the screen to settle.
#[derive(Debug, Clone)]
pub struct SendTermInput {
    /// Target session.
    pub session_id: TermSessionId,
    /// Verbatim text to type (`None` = don't type text).
    pub text: Option<String>,
    /// Named keys to press, in order.
    pub keys: Vec<String>,
    /// Whether to press enter after text/keys.
    pub enter: bool,
    /// How long to wait for output to settle before replying.
    pub max_wait: std::time::Duration,
}

/// Outcome of a [`SendTermInput`] request.
#[derive(Debug, Clone, kameo::Reply)]
pub enum SendTermOutcome {
    /// Input written; screen settled.
    Sent(TermScreen),
    /// The user holds control — nothing was written; the current screen is
    /// returned and the caller must relay the wait notice.
    UserHasControl(TermScreen),
    /// The session id is unknown.
    UnknownSession,
    /// The session already exited; screen plus captured exit info.
    Exited(TermScreen),
}

/// Kill a session (its whole process group) and collect the final state.
#[derive(Debug, Clone)]
pub struct KillTerm {
    /// Target session.
    pub session_id: TermSessionId,
}

/// Outcome of a [`KillTerm`] request.
#[derive(Debug, Clone, kameo::Reply)]
pub enum KillTermOutcome {
    /// The session was killed (or had already exited — kill is idempotent).
    Killed {
        /// Final rendered screen.
        screen: String,
        /// Transcript tail (sequence of observed screens).
        transcript_tail: String,
        /// Captured exit info.
        exited: ExitInfo,
    },
    /// The session id is unknown.
    UnknownSession,
}

/// Flip who holds control of the active session.
///
/// Published by the takeover UI (take-control keybind / handback keybind).
/// Not an `ask` — fire-and-forget with a reply so the sender knows it landed.
#[derive(Debug, Clone)]
pub struct SetTermControl {
    /// The new control holder.
    pub holder: ControlHolder,
}

impl crate::common::bus::BusMessage for SetTermControl {}
