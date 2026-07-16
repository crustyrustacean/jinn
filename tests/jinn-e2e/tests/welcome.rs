//! Cucumber `World` for welcome-plugin e2e tests.
//!
//! Builds a full [`TuiApp`] (rendering disabled) via the shared harness with the
//! `welcome` global plugin copied into the temp config tree. Validates the
//! simplest global plugin: `on_app_started` fires and posts a system entry
//! visible in the startup session history. (No LLM scripting needed — the
//! greeting is unconditional.)

use std::sync::Arc;
use std::time::Duration;

use cucumber::World;
use jinn_domain::ChatEntryKind;
use jinn_domain::{ChatEntry, FakeLlmServiceFactory};
use jinn_tui::TuiApp;

use crate::harness::{build_tuiapp_in_temp, copy_plugin_to_temp};

/// Cucumber world for welcome-plugin scenarios.
///
/// Mirrors [`JudgeWorld`](crate::judge::JudgeWorld)'s shape: holds only what
/// isn't reachable through [`TuiApp`] — the `tuiapp` itself, the typed fake
/// LLM factory (its queueing API is erased behind `Arc<dyn>` inside the app),
/// and the temp dir. Everything else is read off `tuiapp` on demand.
#[derive(World)]
#[world(init = Self::new_welcome_world)]
pub struct WelcomeWorld {
    /// The full app (rendering disabled). Drives the real startup path.
    tuiapp: TuiApp,
    /// Unused — welcome needs no LLM scripting, but the harness signature
    /// requires a factory. Kept to satisfy the build.
    #[allow(dead_code)]
    fake_factory: Arc<FakeLlmServiceFactory>,
    /// Temp directory holding all test filesystem paths. Cleaned up on drop.
    #[allow(dead_code)]
    temp_dir: tempfile::TempDir,
}

impl std::fmt::Debug for WelcomeWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WelcomeWorld").finish_non_exhaustive()
    }
}

impl WelcomeWorld {
    /// Creates a new world via the shared harness: temp dir, welcome global
    /// plugin copied into the config tree, then a real actor system wrapped
    /// in a `TuiApp`.
    async fn new_welcome_world() -> Self {
        let temp_dir = tempfile::TempDir::new().expect("test temp dir");
        copy_plugin_to_temp(temp_dir.path(), "welcome");
        let fake_factory = Arc::new(FakeLlmServiceFactory::new(vec![]));
        let tuiapp = build_tuiapp_in_temp(temp_dir.path(), fake_factory.clone()).await;
        Self {
            tuiapp,
            fake_factory,
            temp_dir,
        }
    }

    /// Polls shared state until `predicate` holds, returning the observed
    /// value before the deadline. Mirrors the polling pattern used by the
    /// other worlds (avoids a fixed sleep racing with the detached startup
    /// hook).
    async fn wait_until<F, T>(&self, predicate: F) -> Option<T>
    where
        F: Fn(&jinn_domain::AppState) -> Option<T>,
    {
        let state = self.tuiapp.core.state.clone();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(v) = predicate(&state.read()) {
                return Some(v);
            }
            if tokio::time::Instant::now() > deadline {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Returns the first matching entry in the active (startup) session
    /// history, waiting up to the deadline for it to appear.
    async fn find_entry(&self, contains: &str) -> Option<ChatEntry> {
        self.wait_until(|s| {
            s.session
                .active_session()
                .history()
                .iter()
                .find(|e| e.text().contains(contains))
                .cloned()
        })
        .await
    }
}

// ─── Step definitions ───────────────────────────────────────────────────

#[cucumber::given(expr = "a fresh app")]
fn given_fresh_app(_world: &mut WelcomeWorld) {
    // The world's constructor already built a fresh app with the welcome
    // plugin loaded. This step exists for Gherkin readability; it is a no-op.
}

#[cucumber::then(expr = "the active session history contains a system entry containing {string}")]
async fn then_history_contains_system_entry(world: &mut WelcomeWorld, substring: String) {
    // Given a fresh app with the welcome global plugin loaded.
    // (world constructed in the given step)

    // When polling the startup session history.
    let entry = world
        .find_entry(&substring)
        .await
        .unwrap_or_else(|| panic!("no history entry contains {substring:?}"));

    // Then the matching entry is a system entry.
    assert!(
        matches!(entry.kind, ChatEntryKind::System(_)),
        "expected System entry, got {:?}",
        entry.kind
    );
}
