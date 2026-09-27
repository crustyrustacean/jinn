//! The export slice — writes a session to a single self-contained file.
//!
//! The `/export` slash command publishes [`jinn_export_msg::ExportSessionToFile`]
//! with the session and whatever path the user typed. This slice picks the
//! format from the resolved path's extension, renders the session, and writes
//! the file — then reports the outcome back into the chat: a transient notice
//! naming the file on success, an `Error` entry on failure.
//!
//! Shape, in dependency order:
//!
//! - [`document`] projects a `SessionSnapshot` into a format-neutral
//!   [`document::ExportDocument`]. It names no output language.
//! - [`format`] holds the [`format::ExportFormat`] trait and the one
//!   extension-to-format resolver. Adding a format is one impl plus one arm.
//! - [`html`] and [`markdown`] are the two formats, each the only module that
//!   knows its own output language.
//! - [`path`] resolves the destination: the default-name ladder, `~`, and
//!   relative paths.
//! - [`export_actor`] drives all of it, doing the slow parts on a blocking
//!   worker with the state lock released.
//!
//! The slice registers no cell, no scope, and no keybind: it owns no UI state
//! and is reachable only by typing `/export`.

pub mod document;
pub mod export_actor;
pub mod format;
pub mod html;
pub mod markdown;
pub mod path;

use jinn_slices::RenderFacts;
use jinn_slices::SliceHost;

use crate::export_actor::ExportActor;
use crate::export_actor::ExportActorDeps;

/// Activates the slice: spawns the export actor on trouper (its
/// `.handles` declaration is the readiness point).
///
/// Nothing else is registered — the slice has no cell, no overlay, and no
/// route rows, so this is the whole activation. The actor reports through the
/// message context's bus, which needs no `Services` handle of its own.
pub fn activate(host: &mut SliceHost<'_, RenderFacts>, state: jinn_kernel::common::state::State) {
    let _ = ExportActor::spawn(host.system(), ExportActorDeps { state });
}
