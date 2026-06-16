//! Process-isolated cucumber runner for the e2e judge tests.
//!
//! Cucumber's default `Runner` executes scenarios sequentially inside a single
//! process. The jinn actor system can't have two coexisting instances in one
//! process (kameo's `ACTOR_REGISTRY` is process-global; `register("env-init")`
//! collides on the second build and hangs forever). So we can't use the default
//! multi-scenario runner.
//!
//! This module splits the binary into two modes selected by argv:
//!
//! - **Parent mode** (`cargo test` / `just test`, no `--scenario`):
//!   Parses `.feature` files, enumerates scenarios, and spawns one **child
//!   process per scenario** concurrently (capped by available parallelism).
//!   Each child's exit code (0 = pass / non-0 = fail) is mapped to a PASS/FAIL
//!   line; the parent exits non-zero if any child failed.
//!
//! - **Child mode** (`--scenario "<feature_path>:<scenario_name>"`):
//!   Runs cucumber with the default runner + step macros, but a **filtering
//!   parser** that emits only the single named scenario. One process = one
//!   scenario = one actor system. No collision, no leak, no global-registry
//!   hack.
//!
//! Gherkin authoring is fully preserved — step functions in `judge.rs` use the
//! `#[given]` / `#[when]` / `#[then]` macros exactly as before. Adding a
//! scenario is just editing a `.feature` file.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use cucumber::gherkin::{self, GherkinEnv};
use cucumber::parser;
use cucumber::{Parser, World as _};
use futures::stream::{self, StreamExt as _};
use tokio::process::Command;

use crate::judge::JudgeWorld;

/// Env var carrying the `"<feature_path>:<scenario_name>"` selector that
/// switches a child process into single-scenario mode. An env var (rather
/// than a CLI arg) avoids colliding with cucumber's own clap parser.
const SCENARIO_ENV: &str = "JINN_E2E_SCENARIO";

/// Entry point. Routes to parent (orchestrator) or child (single scenario) based on argv.
pub async fn run() {
    // The scenario selector is passed via an env var rather than a CLI arg so it
    // never collides with cucumber's own clap-based argument parsing.
    if let Some(scenario_spec) = std::env::var(SCENARIO_ENV).ok().filter(|s| !s.is_empty()) {
        run_child(&scenario_spec).await;
    } else {
        run_parent().await;
    }
}

// ─── Parent: subprocess orchestrator ──────────────────────────────────────

/// Parent mode: enumerate scenarios from `tests/features/judge`, spawn one child
/// per scenario, report PASS/FAIL, exit non-zero if any failed.
async fn run_parent() {
    let features_dir = feature_dir();
    let scenarios = enumerate_scenarios(&features_dir);

    if scenarios.is_empty() {
        eprintln!("[judge-e2e] no scenarios found in {}", features_dir.display());
        std::process::exit(1);
    }

    let current_exe =
        std::env::current_exe().expect("current_exe for child spawning");

    // Cap concurrency at available parallelism so we don't boot 50 actor systems
    // at once on an 8-core laptop.
    let jobs = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);

    let total = scenarios.len();
    eprintln!(
        "[judge-e2e] running {total} scenario(s) across up to {jobs} child process(es)"
    );

    let results: Vec<(ScenarioSpec, bool)> =
        stream::iter(scenarios.into_iter().map(|spec| {
            let exe = current_exe.clone();
            async move {
                let passed = spawn_child(&exe, &spec).await;
                (spec, passed)
            }
        }))
        // `buffer_unordered(jobs)` runs at most `jobs` children concurrently.
        .buffer_unordered(jobs)
        .collect()
        .await;

    let mut failures = 0usize;
    for (spec, passed) in &results {
        let status = if *passed { "PASS" } else { "FAIL" };
        eprintln!("[judge-e2e] {status}: {}:{}", spec.feature_file_name(), spec.scenario);
        if !passed {
            failures += 1;
        }
    }

    if failures > 0 {
        eprintln!(
            "[judge-e2e] {failures}/{total} scenario(s) FAILED"
        );
        std::process::exit(1);
    }
    eprintln!("[judge-e2e] {total}/{total} scenario(s) passed");
}

/// Spawns a child process for a single scenario and returns whether it passed.
async fn spawn_child(exe: &Path, spec: &ScenarioSpec) -> bool {
    let spec_str = format!("{}:{}", spec.feature_path.display(), spec.scenario);
    let mut cmd = Command::new(exe);
    // Pass the selector via env var (not a CLI arg) to avoid cucumber's clap parser.
    cmd.env(SCENARIO_ENV, &spec_str);
    // Forward stdout/stderr so child diagnostics (tracing, panic messages) surface.
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::inherit());
    cmd.stderr(Stdio::inherit());

    match cmd.output().await {
        Ok(output) => output.status.success(),
        Err(e) => {
            eprintln!("[judge-e2e] failed to spawn child for {spec_str}: {e}");
            false
        }
    }
}

/// Resolves the `tests/features/judge` directory relative to the crate manifest.
fn feature_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/features/judge")
}

/// A single scenario to run, identified by its feature file path + scenario name.
#[derive(Clone, Debug)]
struct ScenarioSpec {
    feature_path: PathBuf,
    scenario: String,
}

impl ScenarioSpec {
    fn feature_file_name(&self) -> String {
        self.feature_path
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::to_owned)
            .unwrap_or_else(|| self.feature_path.display().to_string())
    }
}

/// Walks `dir` for `*.feature` files and flattens their scenarios into a list.
fn enumerate_scenarios(dir: &Path) -> Vec<ScenarioSpec> {
    let mut out = Vec::new();
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| {
        panic!("failed to read feature dir {}: {e}", dir.display())
    });
    // Collect + sort for a stable enumeration order.
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "feature"))
        .collect();
    files.sort();

    for path in files {
        let feature = gherkin::Feature::parse_path(&path, GherkinEnv::default())
            .unwrap_or_else(|e| panic!("failed to parse {}: {e}", path.display()));
        for scenario in &feature.scenarios {
            out.push(ScenarioSpec {
                feature_path: path.clone(),
                scenario: scenario.name.clone(),
            });
        }
    }
    // Stable order: by file then scenario name (already sorted by file above;
    // re-sort to be safe across readers).
    out.sort_by(|a, b| {
        a.feature_path
            .cmp(&b.feature_path)
            .then_with(|| a.scenario.cmp(&b.scenario))
    });
    out
}

// ─── Child: single-scenario cucumber run ───────────────────────────────────

/// Child mode: run cucumber with a filtering parser that emits only `spec`.
///
/// `spec` is `"<feature_path>:<scenario_name>"`.
async fn run_child(spec: &str) {
    // Marker written directly to a file BEFORE tracing init, to prove run_child
    // is reached even if the scenario hangs and cucumber buffers stderr.
    {
        let marker = std::env::temp_dir().join(format!("jinn-e2e-marker-{}", std::process::id()));
        let _ = std::fs::write(&marker, format!("run_child reached: {spec}\n"));
    }
    init_tracing();
    tracing::info!(spec = %spec, "judge-e2e child starting");
    let (feature_path, scenario_name) = spec
        .split_once(':')
        .unwrap_or_else(|| panic!("invalid --scenario spec: {spec:?} (expected '<feature_path>:<scenario_name>')"));

    let feature_path = PathBuf::from(feature_path);
    let feature =
        gherkin::Feature::parse_path(&feature_path, GherkinEnv::default())
            .unwrap_or_else(|e| {
                panic!("failed to parse {}: {e}", feature_path.display())
            });

    // Select only the matching scenario. If the name is ambiguous or missing,
    // emit an empty feature — cucumber will report no steps run, surfacing the
    // misconfiguration as a failure rather than a silent skip.
    let filtered: Vec<gherkin::Scenario> = feature
        .scenarios
        .iter()
        .filter(|s| s.name == scenario_name)
        .cloned()
        .collect();

    let single_feature = gherkin::Feature {
        scenarios: filtered,
        ..feature
    };

    // Run cucumber with the filtering parser + default runner/writer.
    // `fail_on_skipped` makes a scenario with no matching steps fail (instead of
    // silently passing), which catches typos in the feature or step patterns.
    JudgeWorld::cucumber::<&str>()
        .with_parser(SingleFeatureParser {
            feature: Arc::new(single_feature),
        })
        .fail_on_skipped()
        .run_and_exit("ignored-by-single-feature-parser")
        .await;
}

/// Parser that emits exactly one pre-parsed feature (used in child mode).
#[derive(Clone, Debug)]
struct SingleFeatureParser {
    feature: Arc<gherkin::Feature>,
}

impl<I> Parser<I> for SingleFeatureParser {
    type Cli = cucumber::cli::Empty;
    type Output =
        stream::Once<std::future::Ready<parser::Result<gherkin::Feature>>>;

    fn parse(
        self,
        _input: I,
        _cli: Self::Cli,
    ) -> Self::Output {
        stream::once(std::future::ready(Ok((*self.feature).clone())))
    }
}
/// Initializes a tracing subscriber for the child process.
///
/// Controlled by `RUST_LOG` (defaults to `info` for this binary). Emits to
/// stderr so it surfaces through the parent's inherited stdio.
fn init_tracing() {
    // Write to a log file instead of stderr: cucumber's writer captures
    // stderr/stdout per-scenario and only flushes on completion, so a hung
    // scenario would trap all diagnostics. A file bypasses that capture.
    use std::fs::OpenOptions;
    let log_path = std::env::temp_dir().join(format!(
        "jinn-e2e-{}.log",
        std::process::id()
    ));
    eprintln!("[judge-e2e] tracing to {}", log_path.display());
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&log_path)
        .expect("open tracing log file");
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                // Suppress kameo's chatty per-message spans; keep jinn + warnings.
                tracing_subscriber::EnvFilter::new(
                    "warn,jinn=debug,jinn_domain=debug,kameo=warn,kameo_actors=off",
                )
            }),
        )
        .with_target(true)
        .with_writer(file)
        .try_init();
}
