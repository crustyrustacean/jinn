//! Commands routed to the [`QuakeBarActor`](super::quake_bar_actor).

use serde::{Deserialize, Serialize};

use crate::BusMessage;

/// Submit the current quake bar input into the command log.
///
/// Emitted by the `IntentHandler` on `<enter>` while the `QuakeBar` scope is
/// active. The [`QuakeBarActor`](super::quake_bar_actor) is the sole subscriber
/// and appends `text` to the command log (which is the only writer of the log).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitQuakeBarCommand {
    /// The submitted line.
    pub text: String,
}

impl BusMessage for SubmitQuakeBarCommand {}
