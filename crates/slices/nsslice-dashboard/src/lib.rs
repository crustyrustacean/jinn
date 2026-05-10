//! Dashboard slice — displays registered actors and their startup status.
//!
//! Everything related to the dashboard element lives here: rendering.

pub mod element;

pub use element::DashboardElement;

use nullslop_component::AppUiRegistry;

/// Register dashboard UI element.
pub fn register(registry: &mut AppUiRegistry) {
    registry.register(Box::new(DashboardElement));
}
