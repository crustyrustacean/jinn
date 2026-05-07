//! Application shutdown component.
//!
//! Responsible for gracefully ending the application when the user requests it.
//! Once triggered, no further command processing occurs.

pub mod handler;

pub(crate) use handler::AppQuitHandler;

use crate::AppBus;
use crate::AppUiRegistry;

/// Register the app quit handler.
pub(crate) fn register(bus: &mut AppBus, _registry: &mut AppUiRegistry) {
    AppQuitHandler.register(bus);
}
