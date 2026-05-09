//! Graceful shutdown coordination for actors.
//!
//! Ensures the application doesn't exit until every running actor has had a
//! chance to clean up. When shutdown is triggered, this component waits for each
//! actor to report completion before allowing the application to proceed with
//! exiting.
//!
//! Phase 5: Handler removed — shutdown tracking will be re-implemented in Phase 7.

pub mod state;

pub use state::ShutdownTrackerState;
