//! Single entry point for all cucumber e2e tests.
//!
//! Runs test worlds sequentially against their respective
//! feature directories:
//!
//! 1. `TuiWorld` → `tests/features/tui/`
//! 2. `ActorWorld` → `tests/features/actor/`
//! 3. `AppWorld` → `tests/features/app/`

mod actor;
mod app;
mod tui;

use actor::ActorWorld;
use app::AppWorld;
use cucumber::World;
use tui::TuiWorld;

#[tokio::main]
async fn main() {
    TuiWorld::run("tests/features/tui").await;
    ActorWorld::run("tests/features/actor").await;
    AppWorld::run("tests/features/app").await;
}
