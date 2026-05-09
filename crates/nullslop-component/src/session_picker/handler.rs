//! Session picker handler (stubbed).
//!
//! Phase 5: This handler will be deleted entirely.
//! Stubbed — SessionNew and SetMode commands were removed from the Command enum.

#![allow(missing_docs)]

use nullslop_component_core::define_handler;
use nullslop_services::Services;

use crate::AppState;

define_handler! {
    pub(crate) struct SessionPickerHandler;

    commands {}

    events {}
}
