//! nullslop-bench — harness benchmarking library.
//!
//! Provides bench task definitions, fixture management, CSV output, and
//! builtin lifecycle handler registration for the bench system.

pub mod ast;
pub mod bench_actor;
pub mod bench_tasks;
pub mod compare;
pub mod csv;
pub mod fixture;
pub mod orchestrator;
pub mod show;
pub mod task;
pub mod tasks;
