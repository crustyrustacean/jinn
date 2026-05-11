//! Status bar — displays the active prompt strategy and current model.
//!
//! A display-only component at the bottom of the screen showing which
//! provider/model is active for the current session.

pub mod element;

pub use element::StatusBarElement;

use crate::common::AppUiRegistry;

/// Register the status bar UI element.
pub fn register(registry: &mut AppUiRegistry) {
    registry.register(Box::new(StatusBarElement));
}
