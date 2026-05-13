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

#[rstest::rstest]
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

#[rstest::rstest]
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

#[rstest::rstest]
fn dir_exists_passes_when_directory_present() {
    let fs = FakeFileSystem::default().with_dir("/tmp/mydir");
    let evaluator = DefaultGuardEvaluator::new(fs, FakeShell::default());

    let guard = GuardExpr::Predicate(GuardPredicate::DirExists {
        path: "/tmp/mydir".to_owned(),
    });
    let result = evaluator.evaluate(&guard, &HashMap::new(), &HashMap::new());

    assert!(result.is_passed());
}

#[rstest::rstest]
fn dir_exists_fails_when_directory_absent() {
    let evaluator = make_evaluator();

    let guard = GuardExpr::Predicate(GuardPredicate::DirExists {
        path: "/tmp/nosuchdir".to_owned(),
    });
    let result = evaluator.evaluate(&guard, &HashMap::new(), &HashMap::new());

    assert!(!result.is_passed());
}

// ---- all combinator ----

#[rstest::rstest]
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

#[rstest::rstest]
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

#[rstest::rstest]
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

#[rstest::rstest]
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

#[rstest::rstest]
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

#[rstest::rstest]
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

#[rstest::rstest]
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

#[rstest::rstest]
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

#[rstest::rstest]
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

#[rstest::rstest]
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

#[rstest::rstest]
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

#[rstest::rstest]
fn value_set_passes_when_variable_nonempty() {
    let evaluator = make_evaluator();
    let vars = HashMap::from([("output_dir".to_owned(), "/tmp/result".to_owned())]);

    let guard = GuardExpr::Predicate(GuardPredicate::ValueSet {
        variable: "output_dir".to_owned(),
    });
    let result = evaluator.evaluate(&guard, &vars, &HashMap::new());

    assert!(result.is_passed());
}

#[rstest::rstest]
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

#[rstest::rstest]
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

#[rstest::rstest]
fn file_hash_matches_fails_when_file_missing() {
    let evaluator = make_evaluator();

    let guard = GuardExpr::Predicate(GuardPredicate::FileHashMatches {
        path: "/missing.txt".to_owned(),
    });
    let result = evaluator.evaluate(&guard, &HashMap::new(), &HashMap::new());

    assert!(!result.is_passed());
}

// ---- None variant ----

#[rstest::rstest]
fn none_guard_always_passes() {
    let evaluator = make_evaluator();
    let result = evaluator.evaluate(&GuardExpr::None, &HashMap::new(), &HashMap::new());
    assert!(result.is_passed());
}

