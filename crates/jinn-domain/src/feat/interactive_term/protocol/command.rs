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
///
/// One terminal per chat session: a spawn for a session that already has a
/// live terminal kills the old one first (reported in the outcome). The
/// terminal overlay and sidebar symbol are keyed by this chat session id;
/// the coordinator's own [`TermSessionId`] stays the model-facing handle.
#[derive(Debug, Clone)]
pub struct SpawnTerm {
    /// The chat session that owns this terminal.
    pub chat_session_id: crate::protocol::SessionId,
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
        /// Set when this spawn killed the chat session's previous terminal —
        /// the caller must surface it so the agent knows the old program died.
        killed_previous: Option<KilledPrevious>,
    },
    /// The command failed to spawn (e.g. binary not found).
    Failed(String),
}

/// What happened to a chat session's previous terminal when a new one took
/// its place (one terminal per chat session).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KilledPrevious {
    /// The killed terminal's session id.
    pub session_id: TermSessionId,
    /// Captured exit info from the kill.
    pub exited: ExitInfo,
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
    /// The user holds control — nothing was written; no screen is returned
    /// (the user's terminal is theirs to read). The caller must fail the
    /// tool call with the wait notice.
    UserHasControl,
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

impl crate::common::bus::BusMessage for SendTermKey {}

/// Resize a session's pty + emulator to the terminal overlay's inner rect.
///
/// Published by the render layer when the terminal overlay's inner rect
/// changes. Fire-and-forget; the actor clamps to sane bounds.
#[derive(Debug, Clone)]
pub struct ResizeTerm {
    /// The chat session whose terminal resizes. `None` is a no-op (the
    /// render layer always names the active session; it never broadcasts).
    pub chat_session_id: Option<crate::protocol::SessionId>,
    /// New size as `(rows, cols)`.
    pub size: (u16, u16),
}

impl crate::common::bus::BusMessage for ResizeTerm {}

/// Forward one key event's bytes to a session's pty (user control mode).
///
/// Fire-and-forget: keystrokes must not queue behind screen settle waits.
#[derive(Debug, Clone)]
pub struct SendTermKey {
    /// Target session.
    pub session_id: TermSessionId,
    /// Encoded key bytes to write.
    pub bytes: Vec<u8>,
}
