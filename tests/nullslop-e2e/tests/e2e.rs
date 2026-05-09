//! Single entry point for all cucumber e2e tests.
//!
//! Runs test worlds sequentially against their respective
//! feature directories:
//!
//! 1. `TuiWorld` → `tests/features/tui/`
//! 2. `ActorWorld` → `tests/features/actor/`
//!
//! Phase 7: Bus tests disabled — bus was deleted.

mod actor;
mod bus;
mod tui;

use actor::ActorWorld;
use cucumber::World;
use tui::TuiWorld;

#[tokio::main]
async fn main() {
    TuiWorld::run("tests/features/tui").await;
    // BusWorld tests disabled — bus was deleted in Phase 7.
    // See Phase 9 in the high-level plan for rewrite.
    ActorWorld::run("tests/features/actor").await;
}
