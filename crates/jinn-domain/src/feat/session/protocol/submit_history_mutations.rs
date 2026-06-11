//! Command for workers to submit history mutation batches.
//!
//! Workers produce `Vec<HistoryMutation>` batches and send them via this
//! command. The session actor queues them in `pending_mutations` for
//! application at the next safe drain point (tool batch completion or
//! stream completion).

use serde::{Deserialize, Serialize};

use crate::feat::session::history_mutation::HistoryMutation;
use crate::protocol::CommandMsg;
use crate::protocol::SessionId;

/// Submit a batch of history mutations for deferred application.
///
/// The session actor queues these in `pending_mutations`. They are applied
/// at the next safe drain point (tool batch completion or stream completion).
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("submit_history_mutations")]
pub struct SubmitHistoryMutations {
    /// The session to apply mutations to.
    pub session_id: SessionId,
    /// The mutation batch. Empty batches are silently ignored.
    pub mutations: Vec<HistoryMutation>,
}

impl crate::common::bus::BusMessage for SubmitHistoryMutations {}
