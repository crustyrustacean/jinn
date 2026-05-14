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
