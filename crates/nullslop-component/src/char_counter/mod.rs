//! Character counter display.
//!
//! Shows a live count of how many characters the user has typed in the input box.
//! This is a display-only component — it does not handle any user actions or events.

pub mod element;

pub use element::CharCounterElement;

use crate::AppBus;
use crate::AppUiRegistry;

/// Register the character counter UI element.
pub(crate) fn register(_bus: &mut AppBus, registry: &mut AppUiRegistry) {
    registry.register(Box::new(CharCounterElement));
}
