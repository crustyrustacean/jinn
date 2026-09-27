//! Shared service-status vocabulary for the dashboard.
//!
//! [`ServiceStatusUpdate`] is the generic projection any feature publishes
//! onto its dashboard row. It lives in the kernel surface layer (not in the
//! dashboard crate) because the *owning* features publish it — including
//! kernel-resident ones — while the dashboard canvas actor merely folds it.
//! The dashboard never needs to know a feature exists; a feature never needs
//! to depend on the dashboard.
//!
//! Note that a feature describes its row and nothing more. There is no
//! lifecycle field to set: whether an actor is alive is the runtime's
//! verdict, announced on its own schema, and a feature that could contradict
//! it would be able to report a crash it never observed.

use serde::Deserialize;
use serde::Serialize;

/// How loudly a feature's own note should read.
///
/// A feature cannot set its row's state — that is the runtime's to report —
/// but it can say how much attention its note deserves. The dashboard view
/// maps a tone to a theme token, never to a raw colour, so a feature cannot
/// hardcode a colour that fights the user's theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum NoteTone {
    /// Ordinary status. The default, and what a row reads when the owning
    /// feature expresses no opinion.
    #[default]
    Muted,
    /// Something is degraded but expected to resolve on its own.
    Warning,
    /// Something failed.
    Error,
}

/// A service's status for the dashboard, published by the owning feature.
///
/// Generic projection onto a dashboard row: `description: None` preserves
/// any existing description, and a row that has not been announced by the
/// runtime yet is created as `Running` — the feature naming a row is a claim
/// the actor exists, and the runtime's own announcement corrects it if that
/// claim was wrong. Features translate their service-specific state into this
/// event so the dashboard never needs to know a feature exists.
///
/// This is a bridge-crossing type: the forward relay serializes it onto
/// `jinn.fabric` as a schema broadcast, so the canvas actor's
/// subscription decodes it.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Event)]
#[schema(
    description = "A feature's projection onto its dashboard row (optional description, status message, note tone)."
)]
pub struct ServiceStatusUpdate {
    /// The dashboard row name (the key the owning feature publishes under,
    /// e.g. its `spawn_tracked!`/actor name).
    pub name: String,
    /// New row description; `None` preserves the existing one.
    pub description: Option<String>,
    /// Free-form status message for the Notes column.
    pub status_message: Option<String>,
    /// How loudly to render the note; `None` reads as [`NoteTone::Muted`].
    pub note_tone: Option<NoteTone>,
}

impl crate::BusMessage for ServiceStatusUpdate {}
