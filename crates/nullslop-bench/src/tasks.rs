//! Bench task definitions.
//!
//! Add new tasks here. Each task is a [`BenchTask`] with a prompt,
//! optional fixtures, tool configuration, and a verification function.
//!
//! Task categories:
//! - **1-shot**: Single message, model produces output from scratch.
//! - **fix-code**: Model receives broken code and must fix it.
//! - **redirect**: Multi-turn, model is asked to do X, then told to do Y instead.

#![allow(clippy::missing_docs_in_private_items, reason = "helpers")]

use std::path::Path;
use std::time::Duration;

use crate::task::{BenchTask, BenchTools, CheckResult, VerificationReport};

/// Returns all benchmark tasks.
pub fn bench_tasks() -> Vec<BenchTask> {
    let mut tasks = Vec::new();
    tasks.extend(one_shot_tasks());
    tasks.extend(fix_code_tasks());
    tasks.extend(redirect_tasks());
    tasks
}

// ── 1-shot tasks ─────────────────────────────────────────────────────────

fn one_shot_tasks() -> Vec<BenchTask> {
    vec![
        BenchTask {
            name: "hello-world",
            messages: vec![
                "Write a hello world program in Rust. Save it to src/main.rs and run it.",
            ],
            fixture_dir: Some("hello-world"),
            timeout: Duration::from_secs(300),
            persona: None,
            tools: BenchTools {
                builtins: vec!["bash", "read", "write"],
                custom: vec![],
            },
            verify: verify_hello_world,
        },
        BenchTask {
            name: "json-parser",
            messages: vec![
                "Write a Rust program that parses a JSON file containing an array of \
                 objects with \"name\" (string) and \"age\" (number) fields, then prints \
                 each person's name and age. Create a test file at input.json with at \
                 least 3 people, save the program to src/main.rs, and run it.",
            ],
            fixture_dir: None,
            timeout: Duration::from_secs(300),
            persona: None,
            tools: BenchTools {
                builtins: vec!["bash", "read", "write"],
                custom: vec![],
            },
            verify: verify_json_parser,
        },
        BenchTask {
            name: "word-frequency",
            messages: vec![
                "Create a text file called input.txt with a few sentences of your choice. \
                 Then write a Rust program (src/main.rs) that reads input.txt, counts word \
                 frequencies, and prints the top 5 most common words with their counts. \
                 Run the program.",
            ],
            fixture_dir: None,
            timeout: Duration::from_secs(300),
            persona: None,
            tools: BenchTools {
                builtins: vec!["bash", "read", "write"],
                custom: vec![],
            },
            verify: verify_word_frequency,
        },
        BenchTask {
            name: "http-server",
            messages: vec![
                "Write a minimal HTTP server in Rust (src/main.rs) that listens on \
                 127.0.0.1:18091 and responds to GET / with \"ok\". Do NOT start the \
                 server, just compile-check it with `cargo check`.",
            ],
            fixture_dir: None,
            timeout: Duration::from_secs(300),
            persona: None,
            tools: BenchTools {
                builtins: vec!["bash", "read", "write"],
                custom: vec![],
            },
            verify: verify_http_server,
        },
        BenchTask {
            name: "markdown-to-html",
            messages: vec![
                "Write a Rust program (src/main.rs) that converts a markdown file to HTML. \
                 Create a file called input.md with some markdown content (headings, bold, \
                 italic, a list, and a code block). The program should read input.md and \
                 write output.html. Run the program.",
            ],
            fixture_dir: None,
            timeout: Duration::from_secs(300),
            persona: None,
            tools: BenchTools {
                builtins: vec!["bash", "read", "write"],
                custom: vec![],
            },
            verify: verify_markdown_to_html,
        },
    ]
}

// ── Fix-code tasks ───────────────────────────────────────────────────────

fn fix_code_tasks() -> Vec<BenchTask> {
    vec![
        BenchTask {
            name: "fix-syntax-broken-rust",
            messages: vec![
                "There is a syntax error in src/main.rs. Find and fix it, then run the \
                 program with `cargo run` to confirm it prints the correct sum (15).",
            ],
            fixture_dir: Some("fix-syntax-broken-rust"),
            timeout: Duration::from_secs(300),
            persona: None,
            tools: BenchTools {
                builtins: vec!["bash", "read", "write"],
                custom: vec![],
            },
            verify: verify_fix_syntax_rust,
        },
        BenchTask {
            name: "fix-syntax-broken-python",
            messages: vec![
                "There are syntax errors in main.py. Find and fix them all, then run the \
                 program with `python main.py` to confirm it prints the fibonacci sequence \
                 from fib(0) to fib(9).",
            ],
            fixture_dir: Some("fix-syntax-broken-python"),
            timeout: Duration::from_secs(300),
            persona: None,
            tools: BenchTools {
                builtins: vec!["bash", "read", "write"],
                custom: vec![],
            },
            verify: verify_fix_syntax_python,
        },
        BenchTask {
            name: "fix-logic-fizzbuzz",
            messages: vec![
                "The FizzBuzz program in src/main.rs has a logic bug — it never prints \
                 \"FizzBuzz\". Find and fix the bug, then run the program. The correct \
                 output for 15 should be \"FizzBuzz\", not \"Fizz\".",
            ],
            fixture_dir: Some("fix-logic-fizzbuzz"),
            timeout: Duration::from_secs(300),
            persona: None,
            tools: BenchTools {
                builtins: vec!["bash", "read", "write"],
                custom: vec![],
            },
            verify: verify_fix_logic_fizzbuzz,
        },
        BenchTask {
            name: "fix-logic-sort",
            messages: vec![
                "The bubble sort in src/main.rs has a bug that causes an index out of \
                 bounds panic. Find and fix it, then run the program to confirm it sorts \
                 correctly.",
            ],
            fixture_dir: Some("fix-logic-sort"),
            timeout: Duration::from_secs(300),
            persona: None,
            tools: BenchTools {
                builtins: vec!["bash", "read", "write"],
                custom: vec![],
            },
            verify: verify_fix_logic_sort,
        },
    ]
}

// ── Redirect tasks ───────────────────────────────────────────────────────

fn redirect_tasks() -> Vec<BenchTask> {
    vec![
        BenchTask {
            name: "redirect-change-color",
            messages: vec![
                "Change the background color of index.html from blue to red.",
                "Actually, I changed my mind — make the background dark gray (#333333) \
                 instead and change the heading color to orange.",
            ],
            fixture_dir: Some("redirect-change-color"),
            timeout: Duration::from_secs(300),
            persona: None,
            tools: BenchTools {
                builtins: vec!["bash", "read", "write"],
                custom: vec![],
            },
            verify: verify_redirect_change_color,
        },
        BenchTask {
            name: "redirect-refactor-function",
            messages: vec![
                "Add a function to main.py called calculate_volume that takes length, \
                 width, and height, and returns the volume. Update main() to also print \
                 the volume for each room, assuming a ceiling height of 2.5m.",
                "Wait, I don't need volume. Remove calculate_volume and instead add a \
                 function called calculate_paint_needed that estimates paint in liters \
                 (area * 0.1 liters per square meter for walls). Print that instead.",
            ],
            fixture_dir: Some("redirect-refactor-function"),
            timeout: Duration::from_secs(300),
            persona: None,
            tools: BenchTools {
                builtins: vec!["bash", "read", "write"],
                custom: vec![],
            },
            verify: verify_redirect_refactor,
        },
        BenchTask {
            name: "redirect-switch-language",
            messages: vec![
                "Add a feature to main.py that counts unique words and prints the count.",
                "Actually, rewrite the entire program in Rust instead. Save it to \
                 src/main.rs. The Rust version should read input.txt and print word \
                 count, character count, and unique word count.",
            ],
            fixture_dir: Some("redirect-switch-language"),
            timeout: Duration::from_secs(300),
            persona: None,
            tools: BenchTools {
                builtins: vec!["bash", "read", "write"],
                custom: vec![],
            },
            verify: verify_redirect_switch_language,
        },
    ]
}

// ── Check helpers ───────────────────────────���────────────────────────────

fn check_file_exists(dir: &Path, name: &str) -> CheckResult {
    let check_name = format!("file_exists({name})");
    let path = dir.join(name);
    if path.is_file() {
        tracing::info!(check = %check_name, "PASSED");
        CheckResult::pass(check_name)
    } else {
        let detail = format!("expected file to exist: {}", path.display());
        tracing::warn!(check = %check_name, %detail, "FAILED");
        CheckResult::fail(check_name, detail)
    }
}

fn check_file_contains(dir: &Path, name: &str, needle: &str) -> CheckResult {
    let check_name = format!("file_contains({name}, {needle:?})");
    let content = std::fs::read_to_string(dir.join(name)).unwrap_or_default();
    if content.contains(needle) {
        tracing::info!(check = %check_name, "PASSED");
        CheckResult::pass(check_name)
    } else {
        let detail = format!(
            "expected {name} to contain {needle:?}, content length: {} bytes",
            content.len()
        );
        tracing::warn!(check = %check_name, %detail, "FAILED");
        CheckResult::fail(check_name, detail)
    }
}

fn check_cargo_check(dir: &Path) -> CheckResult {
    let output = std::process::Command::new("cargo")
        .args(["check"])
        .current_dir(dir)
        .output();

    match output {
        Ok(o) if o.status.success() => {
            tracing::info!(check = "cargo_check", "PASSED");
            CheckResult::pass("cargo_check")
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            let detail = format!(
                "cargo check failed (exit {:?}): {}",
                o.status.code(),
                stderr.trim()
            );
            tracing::warn!(check = "cargo_check", %detail, "FAILED");
            CheckResult::fail("cargo_check", detail)
        }
        Err(e) => {
            let detail = format!("failed to execute cargo check: {e}");
            tracing::warn!(check = "cargo_check", %detail, "FAILED");
            CheckResult::fail("cargo_check", detail)
        }
    }
}

fn check_cargo_run(dir: &Path) -> CheckResult {
    let output = std::process::Command::new("cargo")
        .args(["run"])
        .current_dir(dir)
        .output();

    match output {
        Ok(o) if o.status.success() => {
            tracing::info!(check = "cargo_run", "PASSED");
            CheckResult::pass("cargo_run")
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            let stdout = String::from_utf8_lossy(&o.stdout);
            let detail = format!(
                "cargo run failed (exit {:?})\nstdout: {}\nstderr: {}",
                o.status.code(),
                stdout.trim(),
                stderr.trim()
            );
            tracing::warn!(check = "cargo_run", %detail, "FAILED");
            CheckResult::fail("cargo_run", detail)
        }
        Err(e) => {
            let detail = format!("failed to execute cargo run: {e}");
            tracing::warn!(check = "cargo_run", %detail, "FAILED");
            CheckResult::fail("cargo_run", detail)
        }
    }
}

fn check_python_run(dir: &Path, script: &str) -> CheckResult {
    let check_name = format!("python_run({script})");
    let output = std::process::Command::new("python3")
        .arg(script)
        .current_dir(dir)
        .output();

    match output {
        Ok(o) if o.status.success() => {
            tracing::info!(check = %check_name, "PASSED");
            CheckResult::pass(check_name)
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            let stdout = String::from_utf8_lossy(&o.stdout);
            let detail = format!(
                "python3 {script} failed (exit {:?})\nstdout: {}\nstderr: {}",
                o.status.code(),
                stdout.trim(),
                stderr.trim()
            );
            tracing::warn!(check = %check_name, %detail, "FAILED");
            CheckResult::fail(check_name, detail)
        }
        Err(e) => {
            let detail = format!("failed to execute python3 {script}: {e}");
            tracing::warn!(check = %check_name, %detail, "FAILED");
            CheckResult::fail(check_name, detail)
        }
    }
}

/// Check that stdout from a `cargo run` contains an expected string.
fn check_cargo_run_contains(dir: &Path, expected: &str) -> CheckResult {
    let check_name = format!("cargo_run_contains({expected:?})");
    let output = std::process::Command::new("cargo")
        .args(["run"])
        .current_dir(dir)
        .output();

    match output {
        Ok(o) if !o.status.success() => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            let detail = format!(
                "cargo run failed (exit {:?}): {}",
                o.status.code(),
                stderr.trim()
            );
            tracing::warn!(check = %check_name, %detail, "FAILED");
            CheckResult::fail(check_name, detail)
        }
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            if stdout.contains(expected) {
                tracing::info!(check = %check_name, "PASSED");
                CheckResult::pass(check_name)
            } else {
                let detail = format!(
                    "expected stdout to contain {expected:?}\nactual stdout: {}",
                    stdout.trim()
                );
                tracing::warn!(check = %check_name, %detail, "FAILED");
                CheckResult::fail(check_name, detail)
            }
        }
        Err(e) => {
            let detail = format!("failed to execute cargo run: {e}");
            tracing::warn!(check = %check_name, %detail, "FAILED");
            CheckResult::fail(check_name, detail)
        }
    }
}

/// Check that stdout from a `python3` run contains an expected string.
fn check_python_run_contains(dir: &Path, script: &str, expected: &str) -> CheckResult {
    let check_name = format!("python_run_contains({script}, {expected:?})");
    let output = std::process::Command::new("python3")
        .arg(script)
        .current_dir(dir)
        .output();

    match output {
        Ok(o) if !o.status.success() => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            let detail = format!(
                "python3 {script} failed (exit {:?}): {}",
                o.status.code(),
                stderr.trim()
            );
            tracing::warn!(check = %check_name, %detail, "FAILED");
            CheckResult::fail(check_name, detail)
        }
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            if stdout.contains(expected) {
                tracing::info!(check = %check_name, "PASSED");
                CheckResult::pass(check_name)
            } else {
                let detail = format!(
                    "expected stdout to contain {expected:?}\nactual stdout: {}",
                    stdout.trim()
                );
                tracing::warn!(check = %check_name, %detail, "FAILED");
                CheckResult::fail(check_name, detail)
            }
        }
        Err(e) => {
            let detail = format!("failed to execute python3 {script}: {e}");
            tracing::warn!(check = %check_name, %detail, "FAILED");
            CheckResult::fail(check_name, detail)
        }
    }
}

/// Custom check for FizzBuzz: verify that stdout contains "FizzBuzz" as a
/// standalone line (not just as part of another word).
fn check_fizzbuzz_output(dir: &Path) -> CheckResult {
    let output = std::process::Command::new("cargo")
        .args(["run"])
        .current_dir(dir)
        .output();

    match output {
        Ok(o) if !o.status.success() => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            let detail = format!(
                "cargo run failed (exit {:?}): {}",
                o.status.code(),
                stderr.trim()
            );
            tracing::warn!(check = "fizzbuzz_output", %detail, "FAILED");
            CheckResult::fail("fizzbuzz_output", detail)
        }
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            for line in stdout.lines() {
                if line.trim() == "FizzBuzz" {
                    tracing::info!(check = "fizzbuzz_output", "PASSED");
                    return CheckResult::pass("fizzbuzz_output");
                }
            }
            let detail = format!(
                "expected \"FizzBuzz\" as a standalone line\nactual stdout:\n{}",
                stdout.trim()
            );
            tracing::warn!(check = "fizzbuzz_output", %detail, "FAILED");
            CheckResult::fail("fizzbuzz_output", detail)
        }
        Err(e) => {
            let detail = format!("failed to execute cargo run: {e}");
            tracing::warn!(check = "fizzbuzz_output", %detail, "FAILED");
            CheckResult::fail("fizzbuzz_output", detail)
        }
    }
}

// ── Verification functions ───────────────────────────────────────────────

// -- 1-shot verifiers --

fn verify_hello_world(dir: &Path) -> VerificationReport {
    let checks = vec![
        check_file_exists(dir, "src/main.rs"),
        check_cargo_check(dir),
    ];
    VerificationReport::new("hello-world", checks)
}

fn verify_json_parser(dir: &Path) -> VerificationReport {
    let checks = vec![
        check_file_exists(dir, "src/main.rs"),
        check_file_exists(dir, "input.json"),
        check_cargo_check(dir),
    ];
    VerificationReport::new("json-parser", checks)
}

fn verify_word_frequency(dir: &Path) -> VerificationReport {
    let checks = vec![
        check_file_exists(dir, "src/main.rs"),
        check_file_exists(dir, "input.txt"),
        check_cargo_check(dir),
    ];
    VerificationReport::new("word-frequency", checks)
}

fn verify_http_server(dir: &Path) -> VerificationReport {
    let checks = vec![
        check_file_exists(dir, "src/main.rs"),
        // Just needs to compile — we told it NOT to run.
        check_cargo_check(dir),
    ];
    VerificationReport::new("http-server", checks)
}

fn verify_markdown_to_html(dir: &Path) -> VerificationReport {
    let checks = vec![
        check_file_exists(dir, "src/main.rs"),
        check_file_exists(dir, "input.md"),
        check_file_exists(dir, "output.html"),
        check_cargo_check(dir),
    ];
    VerificationReport::new("markdown-to-html", checks)
}

// -- fix-code verifiers --

fn verify_fix_syntax_rust(dir: &Path) -> VerificationReport {
    let checks = vec![check_cargo_check(dir)];
    VerificationReport::new("fix-syntax-broken-rust", checks)
}

fn verify_fix_syntax_python(dir: &Path) -> VerificationReport {
    let checks = vec![
        check_python_run(dir, "main.py"),
        check_python_run_contains(dir, "main.py", "0"),
        check_python_run_contains(dir, "main.py", "34"),
    ];
    VerificationReport::new("fix-syntax-broken-python", checks)
}

fn verify_fix_logic_fizzbuzz(dir: &Path) -> VerificationReport {
    let checks = vec![check_fizzbuzz_output(dir)];
    VerificationReport::new("fix-logic-fizzbuzz", checks)
}

fn verify_fix_logic_sort(dir: &Path) -> VerificationReport {
    let checks = vec![
        check_cargo_run(dir),
        check_cargo_run_contains(dir, "11"),
        check_cargo_run_contains(dir, "90"),
        check_cargo_run_contains(dir, "Sorted"),
    ];
    VerificationReport::new("fix-logic-sort", checks)
}

// -- redirect verifiers --

fn verify_redirect_change_color(dir: &Path) -> VerificationReport {
    let content = std::fs::read_to_string(dir.join("index.html")).unwrap_or_default();

    // Final state: background should be dark gray (#333333), heading should be orange.
    let bg_ok = content.contains("#333")
        || content.contains("#333333")
        || content.contains("darkgray")
        || content.contains("dark gray");
    let heading_ok =
        content.contains("orange") || content.contains("#ff") || content.contains("#FF");

    let checks = vec![
        if bg_ok {
            CheckResult::pass("background_dark_gray")
        } else {
            CheckResult::fail(
                "background_dark_gray",
                "expected background color #333/#333333/darkgray",
            )
        },
        if heading_ok {
            CheckResult::pass("heading_orange")
        } else {
            CheckResult::fail("heading_orange", "expected heading color orange")
        },
    ];
    VerificationReport::new("redirect-change-color", checks)
}

fn verify_redirect_refactor(dir: &Path) -> VerificationReport {
    let content = std::fs::read_to_string(dir.join("main.py")).unwrap_or_default();

    // Should have calculate_paint_needed (or similar paint function), NOT calculate_volume.
    let has_paint = content.contains("paint")
        || content.contains("Paint")
        || content.contains("liter")
        || content.contains("litre");
    let no_volume = !content.contains("volume") && !content.contains("Volume");

    let checks = vec![
        if has_paint {
            CheckResult::pass("has_paint_reference")
        } else {
            CheckResult::fail(
                "has_paint_reference",
                "expected paint/liter reference in main.py",
            )
        },
        if no_volume {
            CheckResult::pass("no_volume_reference")
        } else {
            CheckResult::fail(
                "no_volume_reference",
                "expected no volume/Volume reference in main.py",
            )
        },
    ];
    VerificationReport::new("redirect-refactor-function", checks)
}

fn verify_redirect_switch_language(dir: &Path) -> VerificationReport {
    let checks = vec![
        check_file_exists(dir, "src/main.rs"),
        check_cargo_check(dir),
        check_file_contains(dir, "src/main.rs", "input.txt"),
    ];
    VerificationReport::new("redirect-switch-language", checks)
}
