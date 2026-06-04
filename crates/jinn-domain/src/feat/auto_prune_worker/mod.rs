//! Auto-prune workers — background history pruning strategies.
//!
//! Each worker implements [`HistoryWorker`] and inspects conversation history
//! to identify stale or redundant entries that can be excluded from LLM context.
//!
//! # Adding a new auto-prune strategy
//!
//! 1. Create a new `foo.rs` file in this module.
//! 2. Implement [`HistoryWorker`] for your strategy struct.
//! 3. Spawn a [`HistoryWorkerActor`] with your worker in `actor_wiring.rs`.
//!
//! [`HistoryWorker`]: crate::feat::history_worker::worker_trait::HistoryWorker
//! [`HistoryWorkerActor`]: crate::feat::history_worker::actor::HistoryWorkerActor

pub mod broken_edit;
pub mod consecutive_reads;
pub mod double_edit;
pub mod entry_token_cache;
pub mod read_edit;
pub mod regex;
pub mod todo_prune;
pub mod tool_age_window;
pub mod trivial_assistant;
pub mod user_anchor_radius;


pub use broken_edit::BrokenEditAutoPruneWorker;
pub use consecutive_reads::ConsecutiveReadsAutoPruneWorker;
pub use double_edit::DoubleEditAutoPruneWorker;
pub use entry_token_cache::{
    HistoryWorkerChatEntryTokenCache, HistoryWorkerChatEntryTokenCacheEvictionActor,
    HistoryWorkerChatEntryTokenCacheEvictionActorDeps,
};
pub use read_edit::ReadEditAutoPruneWorker;
pub use regex::RegexAutoPruneWorker;
pub use todo_prune::TodoAutoPruneWorker;
pub use tool_age_window::ToolAgeWindowAutoPruneWorker;
pub use trivial_assistant::TrivialAssistantAutoPruneWorker;
pub use user_anchor_radius::UserAnchorRadiusAutoPruneWorker;
