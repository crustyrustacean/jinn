//! The quake bar's crossing command.
//!
//! [`SubmitQuakeBarCommand`] is the message the input-hook submit action
//! publishes onto the fabric; the route rule (registered at this
//! slice's activation) translates it onto the `jinn.quake-bar` topic,
//! where [`QuakeBarCanvasActor`](crate::canvas_actor::QuakeBarCanvasActor)
//! subscribes. This crate owns the type and its `Schema` definition and
//! exposes the topic constant.

use serde::{Deserialize, Serialize};

use jinn_slices::BusMessage;

/// Submit the current quake bar input into the command log.
///
/// Emitted by the `IntentHandler` on `<enter>` while the `QuakeBar` scope is
/// active. The [`QuakeBarActor`](super::quake_bar_actor) is the sole subscriber
/// and appends `text` to the command log (which is the only writer of the log).
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "Submit the current quake bar input into the command log.")]
pub struct SubmitQuakeBarCommand {
    /// The submitted line.
    pub text: String,
}

impl BusMessage for SubmitQuakeBarCommand {}
