//! App quit handler (stubbed).
//!
//! Phase 5: This handler will be deleted entirely.
//! Stubbed — Quit command was removed from the Command enum.

#![allow(missing_docs)]

use nullslop_component_core::define_handler;
use nullslop_services::Services;

use crate::AppState;

define_handler! {
    pub(crate) struct AppQuitHandler;

    commands {}

    events {}
}
