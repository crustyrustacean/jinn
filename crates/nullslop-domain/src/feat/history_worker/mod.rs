//! History mutation worker infrastructure.
//!
//! Provides the [`HistoryWorker`] trait and [`HistoryWorkerActor`] wrapper.
//! Workers inspect history snapshots and produce declarative mutations
//! that are queued for application at safe points.
//!
//! # Adding a new worker
//!
//! 1. Implement [`HistoryWorker`] for your heuristic type.
//! 2. Spawn a [`HistoryWorkerActor`] with your worker at startup (see `actor_wiring.rs`).
//! 3. The actor automatically subscribes to `HistoryAppended` and submits
//!    mutations via the command bus.

pub mod actor;
pub mod worker_trait;

pub use actor::{HistoryWorkerActor, HistoryWorkerActorDeps};
pub use worker_trait::HistoryWorker;

#[cfg(test)]
mod tests;
