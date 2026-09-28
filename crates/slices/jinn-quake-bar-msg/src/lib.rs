//! Quake bar vocabulary — the cell payload and the slot it lives under.
//!
//! The overlay console's state is registered by the shared cell catalog
//! rather than by the slice at activation, so the payload lives here, in a
//! crate that depends on no slice implementation. The catalog can then name
//! the slot without pulling in the actor, the renderer, or the routes that
//! go with them.

pub mod state;

pub use state::CommandLog;
pub use state::QuakeBarInput;
pub use state::QuakeBarState;
pub use state::quake_bar_slot;
pub use state::quake_scope;
