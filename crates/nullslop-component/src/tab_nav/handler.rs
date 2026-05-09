//! Tab nav handler (stubbed).
//!
//! Phase 5: This handler will be deleted entirely.
//! Stubbed — SwitchTab command was removed from the Command enum.

#![allow(missing_docs)]

use nullslop_component_core::define_handler;
use nullslop_services::Services;

use crate::AppState;

define_handler! {
    pub(crate) struct TabNavHandler;

    commands {}

    events {}
}
