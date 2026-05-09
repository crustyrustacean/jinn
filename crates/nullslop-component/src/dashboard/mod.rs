//! Dashboard component — displays registered actors and their status.
//!
//! Shows a list of all actors known to the application along with their
//! startup lifecycle status ("Starting" or "Running"). The dashboard updates
//! in real-time as actors progress through the startup sequence.
//!
//! Phase 5: Handler removed — dashboard state updates will be re-implemented in Phase 7.

pub(crate) mod element;
pub mod state;

pub use element::DashboardElement;
pub use state::DashboardState;
