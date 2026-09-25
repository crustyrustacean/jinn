//! Dashboard keyboard navigation, routed over the actor bus.
//!
//! `j`/`k`/`g`/`G` on the dashboard tab are commands to the application,
//! not keystroke composition, so they travel the fabric: the keymap
//! produces a `DashboardSelect*` intent, the feature's route row (see
//! [`jinn_slices::KeyRoutes`]) maps it to a
//! [`DashboardNav`], and the bus delivers it to the dashboard actor —
//! its sole subscriber, which folds the navigation into the slice cell.
//!
//! Sole-subscriber note: the fabric is broadcast, so "routing to the
//! dashboard actor" relies on it being the only subscriber for this
//! type. That pairing is asserted by test; keep `DashboardNav` reserved
//! for the dashboard actor's consumption.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use jinn_slices::BusMessage;

/// Move the dashboard's selection cursor.
///
/// Published by the intent router on behalf of the dashboard feature's
/// keybind rows; consumed only by
/// [`DashboardCanvasActor`](super::canvas_actor::DashboardCanvasActor).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DashboardNav {
    /// Move selection up one entry (`k`).
    Up,
    /// Move selection down one entry (`j`).
    Down,
    /// Jump to the first entry (`g`).
    First,
    /// Jump to the last entry (`G`).
    Last,
}

impl BusMessage for DashboardNav {}

impl trouper::schema::Schema for DashboardNav {
    fn schema_def() -> trouper::schema::SchemaDef {
        trouper::schema::SchemaDef {
            name: "DashboardNav".to_owned(),
            kind: trouper::schema::SchemaKind::Command,
            fields: vec![],
            description: Some("Move the dashboard's selection cursor (enum payload).".to_owned()),
        }
    }
}

impl trouper::envelope::PayloadValue for DashboardNav {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn field(&self, _name: &str) -> Option<String> {
        None
    }

    fn to_json_bytes(&self) -> Arc<[u8]> {
        trouper::envelope::payload_value_json_bytes(self)
    }

    fn clone_value(&self) -> Box<dyn trouper::envelope::PayloadValue> {
        Box::new(*self)
    }
}
