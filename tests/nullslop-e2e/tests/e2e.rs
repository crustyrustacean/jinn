//! Single entry point for all cucumber e2e tests.
//!
//! Runs test worlds sequentially against their respective
//! feature directories:
//!
//! 1. `TuiWorld` → `tests/features/tui/`
//! 2. `AppWorld` → `tests/features/app/`
//! 3. `HeadlessWorld` → `tests/features/headless/`

mod app;
mod headless;
mod tui;

use app::AppWorld;
use cucumber::World;
use headless::HeadlessWorld;
use tui::TuiWorld;

#[tokio::main]
async fn main() {
    TuiWorld::run("tests/features/tui").await;
    AppWorld::run("tests/features/app").await;
    HeadlessWorld::run("tests/features/headless").await;
}
