//! Dashboard crossing contracts.
//!
//! The EXPORT surface of the dashboard slice: the tab's cell payload, the
//! row model it holds, and the slot key that cell is registered under.
//! These live here rather than in `jinn-dashboard` so the shared cell
//! catalog can register the slot without depending on the slice, which
//! depends on the kernel. The slice re-exports all three so its own
//! modules keep reading them from one path.

pub mod state;

pub use state::{DashboardEntry, DashboardState, dashboard_slot};
