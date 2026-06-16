//! Single entry point for e2e tests.
//!
//! Delegates to [`runner`], which implements process isolation: each scenario
//! runs in its own child process so the jinn actor system (and its process-
//! global kameo registry) never has two coexisting instances.
//!
//! Only the judge world is currently enabled. The legacy `app`, `tui`, and
//! `headless` worlds drifted out of sync with the actor system API after the
//! actor-system migration and are intentionally left disabled pending a
//! dedicated fix-up task.

mod judge;
mod runner;

#[tokio::main]
async fn main() {
    runner::run().await;
}
