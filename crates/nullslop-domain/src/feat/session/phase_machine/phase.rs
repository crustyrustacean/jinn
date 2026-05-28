//! Phase types for the session phase machine.
//!
//! [`Phase`] is a struct-per-variant enum where each phase carries its own
//! state. Transitioning away from a variant drops its data automatically.
//!
//! [`PhaseKind`] is the discriminant used for event emission and logging
//! where the per-phase data is not needed.

use std::collections::HashMap;

/// Discriminant of [`Phase`] — used for event emission where phase data is not needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhaseKind {
    Idle,
    Sending,
    Streaming,
    TearingDown,
}

/// No per-phase data needed for Idle.
#[derive(Debug, Clone, Default)]
pub struct IdlePhase;

/// Carries the `tool_loop_disabled` flag, set by judge verdict tools
/// (`task_complete`, `task_incomplete`) to stop the tool loop after
/// the current batch.
#[derive(Debug, Clone, Default)]
pub struct SendingPhase {
    /// When `true`, `on_tool_batch_completed` transitions to `Idle`
    /// instead of continuing the tool loop. Self-clearing on transition.
    pub tool_loop_disabled: bool,
}

/// Carries all streaming tracking state — ephemeral indices and maps
/// that are only meaningful while the LLM is actively streaming tokens.
///
/// All fields are cleared when transitioning away from `Streaming`.
#[derive(Debug, Clone, Default)]
pub struct StreamingPhase {
    /// Index into history for the entry currently receiving stream tokens.
    pub streaming_entry_index: Option<usize>,
    /// Index into history for the entry currently receiving thinking tokens.
    pub streaming_thinking_entry_index: Option<usize>,
    /// Maps stream tool-call index to history index for in-progress tool calls.
    pub streaming_tool_call_indices: HashMap<usize, usize>,
    /// Maps tool_call_id to history index for pending streaming ToolResult entries.
    pub streaming_tool_result_indices: HashMap<String, usize>,
    /// When `true`, the next stream-completion boundary transitions to `Idle`
    /// instead of continuing the tool loop. Set by `soft_cancel()`.
    pub soft_cancel_requested: bool,
}

/// No per-phase data needed for TearingDown.
#[derive(Debug, Clone, Default)]
pub struct TearingDownPhase;

/// The current session phase with per-phase state.
///
/// Each variant carries its own state struct. Transitioning away from a variant
/// drops its data automatically — no manual cleanup of streaming indices or flags.
#[derive(Debug, Clone)]
pub enum Phase {
    /// Session is idle — no LLM request in flight.
    Idle(IdlePhase),
    /// A message has been dispatched to the LLM but no tokens have arrived yet.
    Sending(SendingPhase),
    /// LLM tokens are actively streaming into the session.
    Streaming(StreamingPhase),
    /// A lifecycle teardown script is running.
    TearingDown(TearingDownPhase),
}

impl Phase {
    /// Returns the discriminant without the per-phase data.
    pub fn kind(&self) -> PhaseKind {
        match self {
            Self::Idle(_) => PhaseKind::Idle,
            Self::Sending(_) => PhaseKind::Sending,
            Self::Streaming(_) => PhaseKind::Streaming,
            Self::TearingDown(_) => PhaseKind::TearingDown,
        }
    }
}

impl Default for Phase {
    fn default() -> Self {
        Self::Idle(IdlePhase)
    }
}
