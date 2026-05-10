//! Character counter slice — displays grapheme count for the active input buffer.
//!
//! A display-only component showing how many grapheme clusters the user has typed.
//! Updates in real time as the user types.

pub mod element;

pub use element::CharCounterElement;

use nullslop_component::AppUiRegistry;

/// Register char counter UI element.
pub fn register(registry: &mut AppUiRegistry) {
    registry.register(Box::new(CharCounterElement));
}
