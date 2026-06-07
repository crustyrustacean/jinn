//! Discovery coordinator — coalesces the three per-resource scan events
//! (`SkillsLoaded`, `PromptTemplatesLoaded`, `ContextFilesLoaded`) per session
//! and emits a single [`SessionDiscoverySettled`] when all three have arrived
//! or the 3000ms safety-net timer fires.

pub mod coordinator_actor;
mod session_discovery_settled;

pub use coordinator_actor::{DiscoveryCoordinatorActor, DiscoveryCoordinatorActorDeps};
pub use session_discovery_settled::{DiscoverySnapshot, SessionDiscoverySettled};
