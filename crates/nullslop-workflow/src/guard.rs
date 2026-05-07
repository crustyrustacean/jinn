//! Guard system — composable predicates for step completion verification.
//!
//! Guards are evaluated after a step executes to verify that the step actually
//! produced what it claims. The guard language is a DSL of combinators (`all`,
//! `any`, `not`) wrapping built-in predicates (`file_exists`, `dir_exists`,
//! `file_hash_matches`, `command_succeeds`, `output_matches`, `value_set`).
//!
//! Guard evaluation is synchronous. The async boundary (LLM dispatch, streaming)
//! is handled by the workflow executor actor in Phase 3, which calls into this
//! sync guard library.

use std::collections::HashMap;
use std::io;
use std::process::Output;

use serde::{Deserialize, Serialize};

use crate::template::resolve_template;

/// A built-in guard predicate.
///
/// Each variant represents a single check that can be performed against the
/// filesystem, shell, or runtime state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type")]
pub enum GuardPredicate {
    /// Path resolves to an existing file. Supports glob patterns.
    #[serde(rename = "file_exists")]
    FileExists {
        /// File path (may contain `{{var}}` template variables).
        path: String,
    },

    /// Path resolves to an existing directory.
    #[serde(rename = "dir_exists")]
    DirExists {
        /// Directory path (may contain `{{var}}` template variables).
        path: String,
    },

    /// File content hash matches the stored hash from last completion.
    #[serde(rename = "file_hash_matches")]
    FileHashMatches {
        /// File path (may contain `{{var}}` template variables).
        path: String,
    },

    /// Shell command exits with code 0.
    #[serde(rename = "command_succeeds")]
    CommandSucceeds {
        /// Shell command to run (may contain `{{var}}` template variables).
        command: String,
    },

    /// Shell command stdout matches a regex pattern.
    #[serde(rename = "output_matches")]
    OutputMatches {
        /// Shell command to run.
        command: String,
        /// Regex pattern to match against stdout.
        pattern: String,
    },

    /// A step output variable is non-empty.
    #[serde(rename = "value_set")]
    ValueSet {
        /// Variable name to check.
        variable: String,
    },
}

/// A composable guard expression.
///
/// Guards are built from predicates combined with boolean combinators. The
/// `None` variant represents the absence of guards (always passes).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum GuardExpr {
    /// No guards — always passes. Serializes as `null`.
    #[default]
    None,
    /// A single predicate check.
    Predicate(GuardPredicate),
    /// All child expressions must pass (AND).
    All {
        /// Child guard expressions.
        all: Vec<GuardExpr>,
    },
    /// At least one child expression must pass (OR).
    Any {
        /// Child guard expressions.
        any: Vec<GuardExpr>,
    },
    /// Child expression must fail (NOT).
    Not {
        /// The expression that must fail.
        not: Box<GuardExpr>,
    },
}

/// The result of evaluating a guard expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardResult {
    /// All predicates passed.
    Passed,
    /// One or more predicates failed, with details.
    Failed(Vec<GuardFailure>),
}

impl GuardResult {
    /// Returns `true` if the guard passed.
    pub fn is_passed(&self) -> bool {
        matches!(self, Self::Passed)
    }
}

/// Details about a single guard failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardFailure {
    /// Human-readable reason for the failure.
    pub reason: String,
}

/// Filesystem operations needed by guard evaluation.
///
/// Abstracted behind a trait for testability. Production implementation uses
/// `std::fs`. Test implementations use fakes.
pub trait GuardFileSystem {
    /// Returns `true` if the path resolves to an existing file.
    fn file_exists(&self, path: &str) -> bool;
    /// Returns `true` if the path resolves to an existing directory.
    fn dir_exists(&self, path: &str) -> bool;
    /// Returns the SHA-256 content hash of a file, or `None` if it doesn't exist.
    fn file_hash(&self, path: &str) -> Option<String>;
}

/// Shell execution needed by guard evaluation.
///
/// Abstracted behind a trait for testability. Production implementation uses
/// `std::process::Command`. Test implementations use fakes.
pub trait GuardShell {
    /// Run a shell command and return its output.
    ///
    /// # Errors
    ///
    /// Returns an error if the command cannot be executed.
    fn run_command(&self, command: &str) -> io::Result<Output>;
}

/// Trait for evaluating guard expressions against the filesystem and runtime state.
pub trait GuardEvaluator {
    /// Evaluate a guard expression with the given template variables and stored hashes.
    fn evaluate(
        &self,
        guard: &GuardExpr,
        variables: &HashMap<String, String>,
        stored_hashes: &HashMap<String, String>,
    ) -> GuardResult;
}

// ---------------------------------------------------------------------------
// Production implementations
// ---------------------------------------------------------------------------

/// Production filesystem implementation using `std::fs`.
#[derive(Debug, Clone, Copy)]
pub struct RealFileSystem;

impl GuardFileSystem for RealFileSystem {
    fn file_exists(&self, path: &str) -> bool {
        std::path::Path::new(path).is_file()
    }

    fn dir_exists(&self, path: &str) -> bool {
        std::path::Path::new(path).is_dir()
    }

    fn file_hash(&self, path: &str) -> Option<String> {
        crate::hash::file_content_hash(std::path::Path::new(path))
    }
}

/// Production shell implementation using `std::process::Command`.
#[derive(Debug, Clone, Copy)]
pub struct RealShell;

impl GuardShell for RealShell {
    fn run_command(&self, command: &str) -> io::Result<Output> {
        std::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            .output()
    }
}

// ---------------------------------------------------------------------------
// Default evaluator
// ---------------------------------------------------------------------------

/// Default guard evaluator with pluggable filesystem and shell backends.
#[derive(Debug, Clone)]
pub struct DefaultGuardEvaluator<F, S>
where
    F: GuardFileSystem,
    S: GuardShell,
{
    /// Filesystem backend.
    fs: F,
    /// Shell backend.
    shell: S,
}

impl<F, S> DefaultGuardEvaluator<F, S>
where
    F: GuardFileSystem,
    S: GuardShell,
{
    /// Creates a new evaluator with the given filesystem and shell backends.
    pub fn new(fs: F, shell: S) -> Self {
        Self { fs, shell }
    }
}

impl<F, S> GuardEvaluator for DefaultGuardEvaluator<F, S>
where
    F: GuardFileSystem,
    S: GuardShell,
{
    fn evaluate(
        &self,
        guard: &GuardExpr,
        variables: &HashMap<String, String>,
        stored_hashes: &HashMap<String, String>,
    ) -> GuardResult {
        match guard {
            GuardExpr::None => GuardResult::Passed,

            GuardExpr::Predicate(pred) => self.evaluate_predicate(pred, variables, stored_hashes),

            GuardExpr::All { all } => {
                let mut failures = Vec::new();
                for expr in all {
                    match self.evaluate(expr, variables, stored_hashes) {
                        GuardResult::Passed => {}
                        GuardResult::Failed(f) => failures.extend(f),
                    }
                }
                if failures.is_empty() {
                    GuardResult::Passed
                } else {
                    GuardResult::Failed(failures)
                }
            }

            GuardExpr::Any { any } => {
                let mut all_failures = Vec::new();
                for expr in any {
                    match self.evaluate(expr, variables, stored_hashes) {
                        GuardResult::Passed => return GuardResult::Passed,
                        GuardResult::Failed(f) => all_failures.extend(f),
                    }
                }
                GuardResult::Failed(all_failures)
            }

            GuardExpr::Not { not } => match self.evaluate(not, variables, stored_hashes) {
                GuardResult::Passed => GuardResult::Failed(vec![GuardFailure {
                    reason: "Not: inner guard passed (expected it to fail)".to_owned(),
                }]),
                GuardResult::Failed(_) => GuardResult::Passed,
            },
        }
    }
}

impl<F, S> DefaultGuardEvaluator<F, S>
where
    F: GuardFileSystem,
    S: GuardShell,
{
    /// Evaluates a single predicate after resolving template variables.
    fn evaluate_predicate(
        &self,
        pred: &GuardPredicate,
        variables: &HashMap<String, String>,
        stored_hashes: &HashMap<String, String>,
    ) -> GuardResult {
        match pred {
            GuardPredicate::FileExists { path } => {
                let resolved = resolve_template(path, variables);
                if is_glob_pattern(&resolved) {
                    Self::eval_glob_exists(&resolved)
                } else {
                    self.eval_file_exists(&resolved, path)
                }
            }

            GuardPredicate::DirExists { path } => {
                let resolved = resolve_template(path, variables);
                if self.fs.dir_exists(&resolved) {
                    GuardResult::Passed
                } else {
                    GuardResult::Failed(vec![GuardFailure {
                        reason: format!("directory does not exist: {resolved}"),
                    }])
                }
            }

            GuardPredicate::FileHashMatches { path } => {
                let resolved = resolve_template(path, variables);
                let current = self.fs.file_hash(&resolved);
                match current {
                    Some(hash) => {
                        if let Some(expected) = stored_hashes.get(&resolved) {
                            if &hash == expected {
                                GuardResult::Passed
                            } else {
                                GuardResult::Failed(vec![GuardFailure {
                                    reason: format!(
                                        "file hash mismatch for {resolved}: expected {expected}, got {hash}"
                                    ),
                                }])
                            }
                        } else {
                            // No stored hash — first-time check, passes if file exists.
                            GuardResult::Passed
                        }
                    }
                    None => GuardResult::Failed(vec![GuardFailure {
                        reason: format!("file does not exist for hash check: {resolved}"),
                    }]),
                }
            }

            GuardPredicate::CommandSucceeds { command } => {
                let resolved = resolve_template(command, variables);
                match self.shell.run_command(&resolved) {
                    Ok(output) => {
                        if output.status.success() {
                            GuardResult::Passed
                        } else {
                            let code = output.status.code();
                            let reason = match code {
                                Some(c) => format!("command failed with exit code {c}: {resolved}"),
                                None => format!("command terminated by signal: {resolved}"),
                            };
                            GuardResult::Failed(vec![GuardFailure { reason }])
                        }
                    }
                    Err(e) => GuardResult::Failed(vec![GuardFailure {
                        reason: format!("command execution failed: {resolved} — {e}"),
                    }]),
                }
            }

            GuardPredicate::OutputMatches { command, pattern } => {
                let resolved_cmd = resolve_template(command, variables);
                match self.shell.run_command(&resolved_cmd) {
                    Ok(output) => {
                        let stdout = String::from_utf8_lossy(&output.stdout);
                        if stdout.contains(pattern) {
                            GuardResult::Passed
                        } else {
                            GuardResult::Failed(vec![GuardFailure {
                                reason: format!(
                                    "command output does not match pattern '{pattern}': {resolved_cmd}"
                                ),
                            }])
                        }
                    }
                    Err(e) => GuardResult::Failed(vec![GuardFailure {
                        reason: format!("command execution failed: {resolved_cmd} — {e}"),
                    }]),
                }
            }

            GuardPredicate::ValueSet { variable } => {
                let resolved = resolve_template(variable, variables);
                match variables.get(&resolved) {
                    Some(value) if !value.is_empty() => GuardResult::Passed,
                    _ => GuardResult::Failed(vec![GuardFailure {
                        reason: format!("variable is not set: {resolved}"),
                    }]),
                }
            }
        }
    }

    /// Evaluates a `file_exists` check for a resolved path.
    fn eval_file_exists(&self, resolved: &str, _original: &str) -> GuardResult {
        if self.fs.file_exists(resolved) {
            GuardResult::Passed
        } else {
            GuardResult::Failed(vec![GuardFailure {
                reason: format!("file does not exist: {resolved}"),
            }])
        }
    }

    /// Evaluates a glob pattern check for file existence.
    fn eval_glob_exists(pattern: &str) -> GuardResult {
        match glob::glob(pattern) {
            Ok(mut paths) => {
                if paths.any(|p| p.is_ok()) {
                    GuardResult::Passed
                } else {
                    GuardResult::Failed(vec![GuardFailure {
                        reason: format!("no files match glob pattern: {pattern}"),
                    }])
                }
            }
            Err(e) => GuardResult::Failed(vec![GuardFailure {
                reason: format!("invalid glob pattern: {pattern} — {e}"),
            }]),
        }
    }
}

/// Returns `true` if the path contains glob metacharacters.
fn is_glob_pattern(path: &str) -> bool {
    path.contains('*') || path.contains('?') || path.contains('[')
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Fake filesystem for testing guard evaluation.
    #[derive(Debug, Clone, Default)]
    struct FakeFileSystem {
        files: HashMap<String, String>, // path → content
        dirs: Vec<String>,
    }

    impl FakeFileSystem {
        fn with_file(mut self, path: &str, content: &str) -> Self {
            self.files.insert(path.to_owned(), content.to_owned());
            self
        }

        fn with_dir(mut self, path: &str) -> Self {
            self.dirs.push(path.to_owned());
            self
        }
    }

    impl GuardFileSystem for FakeFileSystem {
        fn file_exists(&self, path: &str) -> bool {
            self.files.contains_key(path)
        }

        fn dir_exists(&self, path: &str) -> bool {
            self.dirs.iter().any(|d| d == path)
        }

        fn file_hash(&self, path: &str) -> Option<String> {
            self.files
                .contains_key(path)
                .then(|| format!("fake-hash-{path}"))
        }
    }

    /// A clonable shell result for testing.
    #[derive(Debug, Clone)]
    enum FakeCommandResult {
        Success { stdout: Vec<u8> },
        Failure { exit_code: i32 },
    }

    /// Fake shell for testing guard evaluation.
    #[derive(Debug, Clone, Default)]
    struct FakeShell {
        results: HashMap<String, FakeCommandResult>,
    }

    impl FakeShell {
        fn with_success(mut self, command: &str, stdout: &str) -> Self {
            self.results.insert(
                command.to_owned(),
                FakeCommandResult::Success {
                    stdout: stdout.as_bytes().to_vec(),
                },
            );
            self
        }

        fn with_exit_code(mut self, command: &str, code: i32) -> Self {
            self.results.insert(
                command.to_owned(),
                FakeCommandResult::Failure { exit_code: code },
            );
            self
        }

        fn make_output(result: &FakeCommandResult) -> Output {
            match result {
                FakeCommandResult::Success { stdout } => Output {
                    status: std::process::ExitStatus::default(),
                    stdout: stdout.clone(),
                    stderr: Vec::new(),
                },
                FakeCommandResult::Failure { exit_code } => {
                    #[cfg(unix)]
                    use std::os::unix::process::ExitStatusExt as _;
                    Output {
                        status: std::process::ExitStatus::from_raw(*exit_code << 8),
                        stdout: Vec::new(),
                        stderr: Vec::new(),
                    }
                }
            }
        }
    }

    impl GuardShell for FakeShell {
        fn run_command(&self, command: &str) -> io::Result<Output> {
            match self.results.get(command) {
                Some(result) => Ok(Self::make_output(result)),
                None => Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "command not mocked",
                )),
            }
        }
    }

    fn make_evaluator() -> DefaultGuardEvaluator<FakeFileSystem, FakeShell> {
        DefaultGuardEvaluator::new(FakeFileSystem::default(), FakeShell::default())
    }

    // ---- file_exists ----

    #[test]
    fn file_exists_passes_when_file_present() {
        // Given a filesystem with a file at /tmp/test.txt.
        let fs = FakeFileSystem::default().with_file("/tmp/test.txt", "hello");
        let evaluator = DefaultGuardEvaluator::new(fs, FakeShell::default());

        // When evaluating file_exists for that path.
        let guard = GuardExpr::Predicate(GuardPredicate::FileExists {
            path: "/tmp/test.txt".to_owned(),
        });
        let result = evaluator.evaluate(&guard, &HashMap::new(), &HashMap::new());

        // Then the guard passes.
        assert!(result.is_passed());
    }

    #[test]
    fn file_exists_fails_when_file_absent() {
        // Given an empty filesystem.
        let evaluator = make_evaluator();

        // When evaluating file_exists for a missing file.
        let guard = GuardExpr::Predicate(GuardPredicate::FileExists {
            path: "/tmp/missing.txt".to_owned(),
        });
        let result = evaluator.evaluate(&guard, &HashMap::new(), &HashMap::new());

        // Then the guard fails.
        assert!(!result.is_passed());
    }

    // ---- dir_exists ----

    #[test]
    fn dir_exists_passes_when_directory_present() {
        let fs = FakeFileSystem::default().with_dir("/tmp/mydir");
        let evaluator = DefaultGuardEvaluator::new(fs, FakeShell::default());

        let guard = GuardExpr::Predicate(GuardPredicate::DirExists {
            path: "/tmp/mydir".to_owned(),
        });
        let result = evaluator.evaluate(&guard, &HashMap::new(), &HashMap::new());

        assert!(result.is_passed());
    }

    #[test]
    fn dir_exists_fails_when_directory_absent() {
        let evaluator = make_evaluator();

        let guard = GuardExpr::Predicate(GuardPredicate::DirExists {
            path: "/tmp/nosuchdir".to_owned(),
        });
        let result = evaluator.evaluate(&guard, &HashMap::new(), &HashMap::new());

        assert!(!result.is_passed());
    }

    // ---- all combinator ----

    #[test]
    fn all_passes_when_all_children_pass() {
        let fs = FakeFileSystem::default()
            .with_file("/a.txt", "a")
            .with_file("/b.txt", "b");
        let evaluator = DefaultGuardEvaluator::new(fs, FakeShell::default());

        let guard = GuardExpr::All {
            all: vec![
                GuardExpr::Predicate(GuardPredicate::FileExists {
                    path: "/a.txt".to_owned(),
                }),
                GuardExpr::Predicate(GuardPredicate::FileExists {
                    path: "/b.txt".to_owned(),
                }),
            ],
        };
        let result = evaluator.evaluate(&guard, &HashMap::new(), &HashMap::new());

        assert!(result.is_passed());
    }

    #[test]
    fn all_fails_when_one_child_fails() {
        let fs = FakeFileSystem::default().with_file("/a.txt", "a");
        let evaluator = DefaultGuardEvaluator::new(fs, FakeShell::default());

        let guard = GuardExpr::All {
            all: vec![
                GuardExpr::Predicate(GuardPredicate::FileExists {
                    path: "/a.txt".to_owned(),
                }),
                GuardExpr::Predicate(GuardPredicate::FileExists {
                    path: "/missing.txt".to_owned(),
                }),
            ],
        };
        let result = evaluator.evaluate(&guard, &HashMap::new(), &HashMap::new());

        assert!(!result.is_passed());
    }

    // ---- any combinator ----

    #[test]
    fn any_passes_when_one_child_passes() {
        let fs = FakeFileSystem::default().with_file("/a.txt", "a");
        let evaluator = DefaultGuardEvaluator::new(fs, FakeShell::default());

        let guard = GuardExpr::Any {
            any: vec![
                GuardExpr::Predicate(GuardPredicate::FileExists {
                    path: "/a.txt".to_owned(),
                }),
                GuardExpr::Predicate(GuardPredicate::FileExists {
                    path: "/missing.txt".to_owned(),
                }),
            ],
        };
        let result = evaluator.evaluate(&guard, &HashMap::new(), &HashMap::new());

        assert!(result.is_passed());
    }

    #[test]
    fn any_fails_when_all_children_fail() {
        let evaluator = make_evaluator();

        let guard = GuardExpr::Any {
            any: vec![
                GuardExpr::Predicate(GuardPredicate::FileExists {
                    path: "/a.txt".to_owned(),
                }),
                GuardExpr::Predicate(GuardPredicate::FileExists {
                    path: "/b.txt".to_owned(),
                }),
            ],
        };
        let result = evaluator.evaluate(&guard, &HashMap::new(), &HashMap::new());

        assert!(!result.is_passed());
    }

    // ---- not combinator ----

    #[test]
    fn not_inverts_pass_to_fail() {
        let fs = FakeFileSystem::default().with_file("/a.txt", "a");
        let evaluator = DefaultGuardEvaluator::new(fs, FakeShell::default());

        let guard = GuardExpr::Not {
            not: Box::new(GuardExpr::Predicate(GuardPredicate::FileExists {
                path: "/a.txt".to_owned(),
            })),
        };
        let result = evaluator.evaluate(&guard, &HashMap::new(), &HashMap::new());

        assert!(!result.is_passed());
    }

    #[test]
    fn not_inverts_fail_to_pass() {
        let evaluator = make_evaluator();

        let guard = GuardExpr::Not {
            not: Box::new(GuardExpr::Predicate(GuardPredicate::FileExists {
                path: "/missing.txt".to_owned(),
            })),
        };
        let result = evaluator.evaluate(&guard, &HashMap::new(), &HashMap::new());

        assert!(result.is_passed());
    }

    // ---- nested compositions ----

    #[test]
    fn nested_all_containing_any() {
        // all([file_exists("/a.txt"), any([file_exists("/b.txt"), file_exists("/c.txt")])])
        // a.txt exists, b.txt missing, c.txt exists → all pass
        let fs = FakeFileSystem::default()
            .with_file("/a.txt", "a")
            .with_file("/c.txt", "c");
        let evaluator = DefaultGuardEvaluator::new(fs, FakeShell::default());

        let guard = GuardExpr::All {
            all: vec![
                GuardExpr::Predicate(GuardPredicate::FileExists {
                    path: "/a.txt".to_owned(),
                }),
                GuardExpr::Any {
                    any: vec![
                        GuardExpr::Predicate(GuardPredicate::FileExists {
                            path: "/b.txt".to_owned(),
                        }),
                        GuardExpr::Predicate(GuardPredicate::FileExists {
                            path: "/c.txt".to_owned(),
                        }),
                    ],
                },
            ],
        };
        let result = evaluator.evaluate(&guard, &HashMap::new(), &HashMap::new());

        assert!(result.is_passed());
    }

    // ---- template variable resolution ----

    #[test]
    fn template_variables_resolved_before_evaluation() {
        let fs = FakeFileSystem::default().with_file("/home/user/notes.md", "content");
        let evaluator = DefaultGuardEvaluator::new(fs, FakeShell::default());

        let guard = GuardExpr::Predicate(GuardPredicate::FileExists {
            path: "{{base_dir}}/notes.md".to_owned(),
        });
        let vars = HashMap::from([("base_dir".to_owned(), "/home/user".to_owned())]);
        let result = evaluator.evaluate(&guard, &vars, &HashMap::new());

        assert!(result.is_passed());
    }

    #[test]
    fn unresolved_variable_causes_guard_failure() {
        let evaluator = make_evaluator();

        let guard = GuardExpr::Predicate(GuardPredicate::FileExists {
            path: "{{unknown}}/notes.md".to_owned(),
        });
        let result = evaluator.evaluate(&guard, &HashMap::new(), &HashMap::new());

        // The guard fails because the unresolved path doesn't exist.
        assert!(!result.is_passed());
    }

    // ---- command_succeeds ----

    #[test]
    fn command_succeeds_passes_on_exit_zero() {
        let shell = FakeShell::default().with_success("echo hello", "hello\n");
        let evaluator = DefaultGuardEvaluator::new(FakeFileSystem::default(), shell);

        let guard = GuardExpr::Predicate(GuardPredicate::CommandSucceeds {
            command: "echo hello".to_owned(),
        });
        let result = evaluator.evaluate(&guard, &HashMap::new(), &HashMap::new());

        // FakeShell returns default ExitStatus which is success (code 0).
        assert!(result.is_passed());
    }

    #[test]
    fn command_succeeds_fails_on_nonzero_exit() {
        let shell = FakeShell::default().with_exit_code("bad-cmd", 1);
        let evaluator = DefaultGuardEvaluator::new(FakeFileSystem::default(), shell);

        let guard = GuardExpr::Predicate(GuardPredicate::CommandSucceeds {
            command: "bad-cmd".to_owned(),
        });
        let result = evaluator.evaluate(&guard, &HashMap::new(), &HashMap::new());

        assert!(!result.is_passed());
    }

    // ---- value_set ----

    #[test]
    fn value_set_passes_when_variable_nonempty() {
        let evaluator = make_evaluator();
        let vars = HashMap::from([("output_dir".to_owned(), "/tmp/result".to_owned())]);

        let guard = GuardExpr::Predicate(GuardPredicate::ValueSet {
            variable: "output_dir".to_owned(),
        });
        let result = evaluator.evaluate(&guard, &vars, &HashMap::new());

        assert!(result.is_passed());
    }

    #[test]
    fn value_set_fails_when_variable_empty() {
        let evaluator = make_evaluator();
        let vars = HashMap::from([("output_dir".to_owned(), String::new())]);

        let guard = GuardExpr::Predicate(GuardPredicate::ValueSet {
            variable: "output_dir".to_owned(),
        });
        let result = evaluator.evaluate(&guard, &vars, &HashMap::new());

        assert!(!result.is_passed());
    }

    // ---- file_hash_matches ----

    #[test]
    fn file_hash_matches_passes_when_no_stored_hash() {
        // First-time check — file exists but no stored hash to compare.
        let fs = FakeFileSystem::default().with_file("/test.txt", "content");
        let evaluator = DefaultGuardEvaluator::new(fs, FakeShell::default());

        let guard = GuardExpr::Predicate(GuardPredicate::FileHashMatches {
            path: "/test.txt".to_owned(),
        });
        let result = evaluator.evaluate(&guard, &HashMap::new(), &HashMap::new());

        assert!(result.is_passed());
    }

    #[test]
    fn file_hash_matches_fails_when_file_missing() {
        let evaluator = make_evaluator();

        let guard = GuardExpr::Predicate(GuardPredicate::FileHashMatches {
            path: "/missing.txt".to_owned(),
        });
        let result = evaluator.evaluate(&guard, &HashMap::new(), &HashMap::new());

        assert!(!result.is_passed());
    }

    // ---- None variant ----

    #[test]
    fn none_guard_always_passes() {
        let evaluator = make_evaluator();
        let result = evaluator.evaluate(&GuardExpr::None, &HashMap::new(), &HashMap::new());
        assert!(result.is_passed());
    }

    // ---- Serde roundtrips ----

    #[test]
    fn guard_expr_none_roundtrips() {
        let expr = GuardExpr::None;
        let json = serde_json::to_string(&expr).unwrap();
        let back: GuardExpr = serde_json::from_str(&json).unwrap();
        assert_eq!(expr, back);
    }

    #[test]
    fn guard_predicate_roundtrips() {
        let pred = GuardPredicate::FileExists {
            path: "/tmp/test.txt".to_owned(),
        };
        let json = serde_json::to_string(&pred).unwrap();
        let back: GuardPredicate = serde_json::from_str(&json).unwrap();
        assert_eq!(pred, back);
    }

    #[test]
    fn guard_expr_all_roundtrips() {
        let expr = GuardExpr::All {
            all: vec![
                GuardExpr::Predicate(GuardPredicate::FileExists {
                    path: "/a.txt".to_owned(),
                }),
                GuardExpr::Predicate(GuardPredicate::DirExists {
                    path: "/dir".to_owned(),
                }),
            ],
        };
        let json = serde_json::to_string(&expr).unwrap();
        let back: GuardExpr = serde_json::from_str(&json).unwrap();
        assert_eq!(expr, back);
    }

    #[test]
    fn guard_expr_nested_roundtrips() {
        let expr = GuardExpr::Any {
            any: vec![
                GuardExpr::All {
                    all: vec![GuardExpr::Predicate(GuardPredicate::FileExists {
                        path: "/a".to_owned(),
                    })],
                },
                GuardExpr::Not {
                    not: Box::new(GuardExpr::Predicate(GuardPredicate::ValueSet {
                        variable: "x".to_owned(),
                    })),
                },
            ],
        };
        let json = serde_json::to_string(&expr).unwrap();
        let back: GuardExpr = serde_json::from_str(&json).unwrap();
        assert_eq!(expr, back);
    }

    #[test]
    fn all_guard_predicates_roundtrip() {
        let predicates = vec![
            GuardPredicate::FileExists {
                path: "/f".to_owned(),
            },
            GuardPredicate::DirExists {
                path: "/d".to_owned(),
            },
            GuardPredicate::FileHashMatches {
                path: "/h".to_owned(),
            },
            GuardPredicate::CommandSucceeds {
                command: "true".to_owned(),
            },
            GuardPredicate::OutputMatches {
                command: "echo hi".to_owned(),
                pattern: "hi".to_owned(),
            },
            GuardPredicate::ValueSet {
                variable: "v".to_owned(),
            },
        ];
        for pred in predicates {
            let json = serde_json::to_string(&pred).unwrap();
            let back: GuardPredicate = serde_json::from_str(&json).unwrap();
            assert_eq!(pred, back);
        }
    }
}
