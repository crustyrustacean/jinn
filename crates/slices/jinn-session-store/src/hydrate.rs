//! Session hydration work and results that cross between the store actor and
//! its hydration worker pool.
//!
//! Startup hydration reads every unarchived session's full history. Doing that
//! inside the store actor's own handler would hold its mailbox for the whole
//! run: an actor processes one message to completion, so a handler that awaits
//! a loop of database reads makes every other message — including the session
//! picker's own query — wait behind the entire history.
//!
//! So the read is dispatched instead: the handler sends one [`HydrateSession`]
//! per session to a pool of workers, and returns immediately. Each worker
//! reads one session off the actor's task and answers with
//! [`HydrateCompleted`], which the store actor applies.
//!
//! These types live in the slice rather than in `jinn-session-store-msg`
//! because the result carries a [`SessionSnapshot`], and `jinn-session-state`
//! — which owns that type — already depends on `jinn-session-store-msg`. Moving
//! them to the msg crate would close a dependency cycle.

use jinn_core_types::SessionId;
use jinn_session_state::SessionSnapshot;
use jinn_slices::BusMessage;
use serde::{Deserialize, Serialize};

/// Load one session's full history, off the store actor's mailbox.
///
/// One of these is dispatched per session at startup, and one per missing tree
/// member during the frozen-node pass. The worker performs the read and
/// publishes the result; the store actor never awaits it.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "Load one session's history for startup or tree hydration.")]
pub struct HydrateSession {
    /// The session to load.
    pub session_id: SessionId,
    /// True when the result becomes a frozen tree node rather than a live session.
    pub frozen: bool,
}

impl BusMessage for HydrateSession {}

/// One session's history has been read, and is ready to be applied.
///
/// Published on every path, including a failed or missing read: the store
/// actor counts completions to know when hydration has finished, so a worker
/// that returned early without publishing would leave the sidebar's hydration
/// indicator up forever.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Event)]
#[schema(description = "A session's history was loaded for startup or tree hydration.")]
pub struct HydrateCompleted {
    /// The session that was loaded.
    pub session_id: SessionId,
    /// True when this result is a frozen tree node rather than a live session.
    pub frozen: bool,
    /// The loaded snapshot; `None` when the session was missing or failed to load.
    pub snapshot: Option<SessionSnapshot>,
}

impl BusMessage for HydrateCompleted {}
