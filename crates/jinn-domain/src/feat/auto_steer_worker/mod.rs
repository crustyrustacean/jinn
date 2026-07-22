//! Auto-steer workers — background history injection strategies.
//!
//! Each worker implements [`HistoryWorker`] and inspects conversation history
//! to decide whether to inject a steering `User` entry nudging the agent.
//!
//! # Adding a new auto-steer strategy
//!
//! 1. Create a new `foo.rs` file in this module.
//! 2. Implement [`HistoryWorker`] for your strategy struct.
//! 3. Spawn a [`HistoryWorkerActor`] with your worker in `actor_wiring.rs`.
//!
//! [`HistoryWorker`]: crate::feat::history_worker::worker_trait::HistoryWorker
//! [`HistoryWorkerActor`]: crate::feat::history_worker::actor::HistoryWorkerActor

pub mod todo_steer;

pub use todo_steer::{TodoAutoSteerConfig, TodoAutoSteerWorker};
