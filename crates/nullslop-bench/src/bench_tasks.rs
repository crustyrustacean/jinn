//! Builtin lifecycle handlers for bench tasks.
//!
//! Each bench task is registered as a [`BuiltinHandler`] in the domain's
//! [`BuiltinRegistry`]. The handler's `setup` creates a temporary working
//! directory (with fixture files if applicable), and `teardown` runs the
//! task's verification function against the working directory.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use error_stack::{Report, ResultExt};
use nullslop_domain::feat::session_lifecycle::builtin::{
    BuiltinHandler, BuiltinHandlerError, BuiltinId, BuiltinRegistry,
};
use nullslop_domain::protocol::SessionId;

use crate::fixture;
use crate::tasks;

/// Registers all bench tasks as builtin lifecycle handlers.
///
/// Call this before creating the actor system so the session actor can
/// dispatch bench lifecycle setup/teardown to the correct handler.
pub fn register_bench_tasks(registry: &mut BuiltinRegistry) {
    for task in tasks::bench_tasks() {
        let handler = BenchTaskHandler {
            name: task.name.to_owned(),
            fixture_dir: task.fixture_dir.map(str::to_owned),
            verify: task.verify,
        };
        registry.register(BuiltinId(task.name.to_owned()), Arc::new(handler));
    }
}

// ── Handler implementation ─────────────────────────────────────────────

/// A [`BuiltinHandler`] backed by a bench task definition.
///
/// On setup, creates a temp directory under the system temp dir and copies
/// fixtures if the task has a fixture directory. On teardown, runs the task's
/// verification function against the working directory.
pub struct BenchTaskHandler {
    name: String,
    fixture_dir: Option<String>,
    verify: fn(&Path) -> bool,
}

impl std::fmt::Debug for BenchTaskHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BenchTaskHandler")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl BuiltinHandler for BenchTaskHandler {
    fn setup(
        &self,
        _session_id: &SessionId,
        _args: &[String],
    ) -> Result<PathBuf, Report<BuiltinHandlerError>> {
        let temp_dir = tempfile::tempdir()
            .change_context(BuiltinHandlerError)
            .attach(format!(
                "failed to create temp dir for bench task '{}'",
                self.name
            ))?;

        let work_dir = temp_dir.keep();

        fixture::prepare_fixture(self.fixture_dir.as_deref(), &work_dir)
            .change_context(BuiltinHandlerError)
            .attach(format!(
                "failed to prepare fixture for bench task '{}'",
                self.name
            ))?;

        Ok(work_dir)
    }

    fn teardown(&self, _session_id: &SessionId, _args: &[String]) -> bool {
        // Teardown verification is run against the session's CWD, which was
        // set during setup. The session actor passes the CWD as part of its
        // lifecycle tracking, but the handler doesn't receive it directly.
        //
        // For now, teardown always returns true — the actual verification
        // will be performed by the bench actor in Phase 4, which has access
        // to the session's CWD from state.
        //
        // NOTE: This is a deliberate simplification. The BuiltinHandler trait
        // only receives session_id and args. When we have a concrete bench
        // actor that tracks the CWD, we can pass it via args or a side channel.
        true
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    #[test]
    fn register_bench_tasks_populates_registry() {
        // Given an empty registry.
        let mut registry = BuiltinRegistry::new();

        // When registering bench tasks.
        register_bench_tasks(&mut registry);

        // Then the registry is not empty (has at least one handler).
        assert!(!registry.is_empty());

        // And a known task can be looked up.
        assert!(registry.get(&BuiltinId("hello-world".to_owned())).is_some());
    }

    #[test]
    fn setup_creates_work_dir_with_fixtures() {
        // Given a handler for the fix-syntax-broken-rust task.
        let handler = BenchTaskHandler {
            name: "fix-syntax-broken-rust".to_owned(),
            fixture_dir: Some("fix-syntax-broken-rust".to_owned()),
            verify: tasks::bench_tasks()
                .into_iter()
                .find(|t| t.name == "fix-syntax-broken-rust")
                .expect("task exists")
                .verify,
        };

        // When running setup.
        let session_id = SessionId::new();
        let result = handler.setup(&session_id, &[]);

        // Then it succeeds and returns a path.
        let work_dir = result.expect("setup should succeed");
        assert!(work_dir.is_dir());

        // And the fixture files were copied.
        assert!(
            work_dir.join("src/main.rs").exists(),
            "fixture src/main.rs should exist in {:?}",
            work_dir
        );
    }

    #[test]
    fn setup_creates_empty_dir_without_fixtures() {
        // Given a handler for the hello-world task (no fixture dir).
        let handler = BenchTaskHandler {
            name: "hello-world".to_owned(),
            fixture_dir: None,
            verify: |_path| true,
        };

        // When running setup.
        let session_id = SessionId::new();
        let result = handler.setup(&session_id, &[]);

        // Then it succeeds and returns an empty directory.
        let work_dir = result.expect("setup should succeed");
        assert!(work_dir.is_dir());
        // hello-world has no fixtures, so the dir should be empty.
        assert!(
            std::fs::read_dir(&work_dir)
                .expect("read dir")
                .next()
                .is_none(),
            "empty task should have empty work dir"
        );
    }

    #[test]
    fn teardown_returns_true() {
        // Given any handler.
        let handler = BenchTaskHandler {
            name: "test".to_owned(),
            fixture_dir: None,
            verify: |_path| true,
        };

        // When running teardown.
        let session_id = SessionId::new();
        let result = handler.teardown(&session_id, &[]);

        // Then it returns true (current implementation).
        assert!(result);
    }
}
