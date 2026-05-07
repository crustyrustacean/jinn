//! Status bar — displays the currently active model and provider.
//!
//! A display-only component at the bottom of the screen showing which
//! provider/model is active for the current session. Shows "no model selected"
//! when no provider has been configured.

pub mod element;

use crate::AppBus;
use crate::AppUiRegistry;

pub use element::StatusBarElement;

/// Register the status bar UI element.
pub(crate) fn register(_bus: &mut AppBus, registry: &mut AppUiRegistry) {
    registry.register(Box::new(StatusBarElement));
}
