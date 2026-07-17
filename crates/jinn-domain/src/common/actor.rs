//! Actor SDK for building jinn actors.
//!
//! Provides shared actor utilities: bus message types and scanning helpers.

pub mod actor_counter;
pub mod actor_name;
pub mod protocol;

pub use actor_counter::ActorCounter;
pub use actor_name::ActorName;

pub mod scan_actor;
pub use scan_actor::NoDirectMsg;
