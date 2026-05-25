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
use crate::task::VerificationReport;
use crate::tasks;
use unicode_segmentation::UnicodeSegmentation;

/// Registers all bench tasks as builtin lifecycle handlers.
///
/// Call this before creating the actor system so the session actor can
/// dispatch bench lifecycle setup/teardown to the correct handler.
pub fn register_bench_tasks(registry: &mut BuiltinRegistry, artifact_dir: Option<&Path>) {
    for task in tasks::bench_tasks() {
        let handler = BenchTaskHandler {
            name: task.name.to_owned(),
            fixture_dir: task.fixture_dir,
            verify: task.verify,
            artifact_dir: artifact_dir.map(std::borrow::ToOwned::to_owned),
        };
        registry.register(BuiltinId(task.name.to_owned()), Arc::new(handler));
    }
}

// ── Handler implementation ─────────────────────────────────────────────

/// A [`BuiltinHandler`] backed by a bench task definition.
///
/// On setup, creates a working directory and copies fixtures if the task has a
/// fixture directory. When `artifact_dir` is set, the directory is created under
/// that path (for post-run inspection). Otherwise, falls back to `tempfile::tempdir()`.
/// On teardown, runs the task's verification function against the working directory.
pub struct BenchTaskHandler {
    /// Task name (for logging and error messages).
    name: String,
    /// Embedded fixture directory.
    fixture_dir: Option<&'static include_dir::Dir<'static>>,
    #[expect(
        dead_code,
        reason = "verify is called by the bench actor, not this handler"
    )]
    /// Verify function for the bench task.
    verify: fn(&Path) -> VerificationReport,
    /// When set, create work directories here instead of /tmp.
    artifact_dir: Option<PathBuf>,
}

impl std::fmt::Debug for BenchTaskHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BenchTaskHandler")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

/// Returns the first 10 characters of the session ID for use in directory names.
/// Full IDs like `s-01923abc-def4-7def-8901-234567890abc` are unwieldy;
/// `s-01923abc` is enough to be unique within a single bench run.
fn session_id_short(id: &SessionId) -> String {
    id.to_string().graphemes(true).take(10).collect()
}

impl BuiltinHandler for BenchTaskHandler {
    fn setup(
        &self,
        session_id: &SessionId,
        _args: &[String],
    ) -> Result<PathBuf, Report<BuiltinHandlerError>> {
        let work_dir = match &self.artifact_dir {
            Some(dir) => {
                let short_id = session_id_short(session_id);
                let work = dir.join(format!("{}-{short_id}", self.name));
                std::fs::create_dir_all(&work)
                    .change_context(BuiltinHandlerError)
                    .attach(format!(
                        "failed to create artifact dir for bench task '{}'",
                        self.name
                    ))?;
                work
            }
            None => {
                let temp_dir = tempfile::tempdir()
                    .change_context(BuiltinHandlerError)
                    .attach(format!(
                        "failed to create temp dir for bench task '{}'",
                        self.name
                    ))?;
                temp_dir.keep()
            }
        };

        fixture::prepare_fixture(self.fixture_dir, &work_dir)
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
    #![allow(clippy::expect_used, reason = "test code")]

    use super::*;

    fn noop_verify(_: &Path) -> VerificationReport {
        VerificationReport::new("test", vec![])
    }

    #[test]
    fn register_bench_tasks_populates_registry() {
        // Given an empty registry.
        let mut registry = BuiltinRegistry::new();

        // When registering bench tasks.
        register_bench_tasks(&mut registry, None);

        // Then the registry is not empty (has at least one handler).
        assert!(!registry.is_empty());

        // And a known task can be looked up.
        assert!(registry.get(&BuiltinId("hello-world".to_owned())).is_some());
    }

    #[test]
    fn setup_creates_work_dir_with_fixtures() {
        // Given a handler for the fix-syntax-broken-rust task.
        let task = tasks::bench_tasks()
            .into_iter()
            .find(|t| t.name == "fix-syntax-broken-rust")
            .expect("task exists");
        let handler = BenchTaskHandler {
            name: "fix-syntax-broken-rust".to_owned(),
            fixture_dir: task.fixture_dir,
            verify: task.verify,
            artifact_dir: None,
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
            "fixture src/main.rs should exist in {work_dir:?}"
        );
    }

    #[test]
    fn setup_creates_empty_dir_without_fixtures() {
        // Given a handler for the hello-world task (no fixture dir).
        let handler = BenchTaskHandler {
            name: "hello-world".to_owned(),
            fixture_dir: None,
            verify: noop_verify,
            artifact_dir: None,
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
            verify: noop_verify,
            artifact_dir: None,
        };

        // When running teardown.
        let session_id = SessionId::new();
        let result = handler.teardown(&session_id, &[]);

        // Then it returns true (current implementation).
        assert!(result);
    }

    #[test]
    fn setup_with_artifact_dir_creates_dir_at_specified_path() {
        // Given an artifact directory.
        let artifact_root = tempfile::TempDir::new().expect("temp dir");
        let handler = BenchTaskHandler {
            name: "hello-world".to_owned(),
            fixture_dir: None,
            verify: noop_verify,
            artifact_dir: Some(artifact_root.path().to_owned()),
        };

        // When running setup.
        let session_id = SessionId::new();
        let result = handler.setup(&session_id, &[]);

        // Then it succeeds and the work dir is under the artifact root.
        let work_dir = result.expect("setup should succeed");
        assert!(
            work_dir.starts_with(artifact_root.path()),
            "work dir {work_dir:?} should be under artifact root {:?}",
            artifact_root.path()
        );
        assert!(work_dir.is_dir());

        // And the directory name contains the task name.
        let dir_name = work_dir.file_name().expect("dir name").to_string_lossy();
        assert!(
            dir_name.starts_with("hello-world-"),
            "dir name {dir_name} should start with hello-world-"
        );
    }
}
