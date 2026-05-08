//! Single entry point for all cucumber e2e tests.
//!
//! Runs all three test worlds sequentially against their respective
//! feature directories:
//!
//! 1. `TuiWorld` → `tests/features/tui/`
//! 2. `BusWorld` → `tests/features/bus/`
//! 3. `ActorWorld` → `tests/features/actor/`

mod actor;
mod bus;
mod tui;

use actor::ActorWorld;
use bus::BusWorld;
use cucumber::World;
use tui::TuiWorld;

#[tokio::main]
async fn main() {
    TuiWorld::run("tests/features/tui").await;
    BusWorld::run("tests/features/bus").await;
    ActorWorld::run("tests/features/actor").await;
}
