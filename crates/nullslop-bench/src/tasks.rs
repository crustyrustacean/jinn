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

use crate::task::{BenchTask, BenchTools};

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
            fixture_dir: None,
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

// ── Verification functions ───────────────────────────────────────────────

fn cargo_check(dir: &Path) -> bool {
    std::process::Command::new("cargo")
        .args(["check"])
        .current_dir(dir)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn file_exists(dir: &Path, name: &str) -> bool {
    dir.join(name).is_file()
}

fn file_contains(dir: &Path, name: &str, needle: &str) -> bool {
    let content = std::fs::read_to_string(dir.join(name)).unwrap_or_default();
    content.contains(needle)
}

// -- 1-shot verifiers --

fn verify_hello_world(dir: &Path) -> bool {
    file_exists(dir, "src/main.rs") && cargo_check(dir)
}

fn verify_json_parser(dir: &Path) -> bool {
    if !file_exists(dir, "src/main.rs") || !file_exists(dir, "input.json") {
        return false;
    }
    cargo_check(dir)
}

fn verify_word_frequency(dir: &Path) -> bool {
    if !file_exists(dir, "src/main.rs") || !file_exists(dir, "input.txt") {
        return false;
    }
    cargo_check(dir)
}

fn verify_http_server(dir: &Path) -> bool {
    if !file_exists(dir, "src/main.rs") {
        return false;
    }
    // Just needs to compile — we told it NOT to run.
    cargo_check(dir)
}

fn verify_markdown_to_html(dir: &Path) -> bool {
    file_exists(dir, "src/main.rs")
        && file_exists(dir, "input.md")
        && file_exists(dir, "output.html")
        && cargo_check(dir)
}

// -- fix-code verifiers --

fn verify_fix_syntax_rust(dir: &Path) -> bool {
    cargo_check(dir)
}

fn verify_fix_syntax_python(dir: &Path) -> bool {
    // Run the fixed python file and check it produces output.
    std::process::Command::new("python3")
        .arg("main.py")
        .current_dir(dir)
        .output()
        .map(|o| {
            let stdout = String::from_utf8_lossy(&o.stdout);
            // Should print fib(0)=0 through fib(9)=34.
            o.status.success() && stdout.contains('0') && stdout.contains("34")
        })
        .unwrap_or(false)
}

fn verify_fix_logic_fizzbuzz(dir: &Path) -> bool {
    let Ok(output) = std::process::Command::new("cargo")
        .args(["run"])
        .current_dir(dir)
        .output()
    else {
        return false;
    };

    if !output.status.success() {
        return false;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Line for 15 must be "FizzBuzz" not "Fizz" or "Buzz".
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed == "Fizz" || trimmed == "Buzz" {
            // Could be from 3, 5, 6, 9, etc. — acceptable.
        }
        if trimmed == "FizzBuzz" {
            return true;
        }
    }
    false
}

fn verify_fix_logic_sort(dir: &Path) -> bool {
    let Ok(output) = std::process::Command::new("cargo")
        .args(["run"])
        .current_dir(dir)
        .output()
    else {
        return false;
    };

    if !output.status.success() {
        return false;
    }

    // Output should contain the sorted array.
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.contains("11") && stdout.contains("90") && stdout.contains("Sorted")
}

// -- redirect verifiers --

fn verify_redirect_change_color(dir: &Path) -> bool {
    let content = std::fs::read_to_string(dir.join("index.html")).unwrap_or_default();

    // Final state: background should be dark gray (#333333), heading should be orange.
    let bg_ok = content.contains("#333")
        || content.contains("#333333")
        || content.contains("darkgray")
        || content.contains("dark gray");
    let heading_ok =
        content.contains("orange") || content.contains("#ff") || content.contains("#FF");

    bg_ok && heading_ok
}

fn verify_redirect_refactor(dir: &Path) -> bool {
    let content = std::fs::read_to_string(dir.join("main.py")).unwrap_or_default();

    // Should have calculate_paint_needed (or similar paint function), NOT calculate_volume.
    let has_paint = content.contains("paint")
        || content.contains("Paint")
        || content.contains("liter")
        || content.contains("litre");
    let no_volume = !content.contains("volume") && !content.contains("Volume");

    has_paint && no_volume
}

fn verify_redirect_switch_language(dir: &Path) -> bool {
    // The final program should be in Rust (src/main.rs exists and compiles).
    // It should NOT be the Python version anymore (or at least the Rust version
    // must be the one that works).
    if !file_exists(dir, "src/main.rs") {
        return false;
    }

    if !cargo_check(dir) {
        return false;
    }

    // Should reference input.txt — the original fixture file.
    file_contains(dir, "src/main.rs", "input.txt")
}
