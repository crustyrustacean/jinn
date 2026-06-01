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

pub mod read_edit;
pub mod todo_prune;

pub use read_edit::ReadEditAutoPruneWorker;
pub use todo_prune::TodoAutoPruneWorker;
