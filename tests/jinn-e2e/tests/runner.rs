//! Process-isolated cucumber runner for the e2e test suites.
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
//!   Parses `.feature` files from every registered suite directory, enumerates
//!   scenarios, and spawns one **child process per scenario** concurrently
//!   (capped by available parallelism). Each child's exit code (0 = pass /
//!   non-0 = fail) is mapped to a PASS/FAIL line; the parent exits non-zero if
//!   any child failed.
//!
//! - **Child mode** (`--scenario "<feature_path>:<scenario_name>"`):
//!   Runs cucumber with the default runner + step macros, but a **filtering
//!   parser** that emits only the single named scenario. One process = one
//!   scenario = one actor system. No collision, no leak, no global-registry
//!   hack. The World is selected by the feature file's parent directory
//!   (`tests/features/<suite>/...` → [`WorldKind`]).
//!
//! Gherkin authoring is fully preserved — step functions in each suite's module
//! use the `#[given]` / `#[when]` / `#[then]` macros exactly as before. Adding
//! a scenario is just editing a `.feature` file. Adding a new suite is
//! registering a [`WorldKind`] variant + its feature subdir + a dispatch arm.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use cucumber::gherkin::{self, GherkinEnv};
use cucumber::parser;
use cucumber::{Parser, World as _};
use futures::stream::{self, StreamExt as _};
use tokio::process::Command;

use crate::gap_analysis::GapAnalysisWorld;
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

// ─── Suites: feature dir ↔ World dispatch ─────────────────────────────────

/// A registered e2e suite: maps a feature subdirectory under `tests/features`
/// to the Cucumber [`World`](cucumber::World) that owns its step definitions.
///
/// Each variant corresponds to exactly one suite module + one feature
/// subdirectory. Cucumber collects step definitions per-World (via
/// `WorldInventory`), so `JudgeWorld::cucumber()` only matches judge steps and
/// `GapAnalysisWorld::cucumber()` only matches gap-analysis steps — there is no
/// cross-suite step collision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WorldKind {
    Judge,
    GapAnalysis,
}

impl WorldKind {
    /// All registered suites, in enumeration order.
    const ALL: &'static [Self] = &[Self::Judge, Self::GapAnalysis];

    /// The subdirectory under `tests/features` holding this suite's `.feature`
    /// files.
    const fn feature_subdir(self) -> &'static str {
        match self {
            Self::Judge => "judge",
            Self::GapAnalysis => "gap-analysis",
        }
    }

    /// Resolves the `tests/features/<subdir>` directory relative to the crate
    /// manifest, canonicalized so the absolute path is forwarded to child
    /// processes (which may inherit a different CWD than the parent).
    ///
    /// Returns `None` if the directory does not exist yet — a registered suite
    /// with no `.feature` files is silently skipped at enumeration time rather
    /// than crashing the parent.
    fn feature_dir(self, manifest_dir: &str) -> Option<PathBuf> {
        PathBuf::from(manifest_dir)
            .join("tests/features")
            .join(self.feature_subdir())
            .canonicalize()
            .ok()
    }

    /// Derives the suite owning a `.feature` file from its parent directory
    /// name. Returns `None` if the directory isn't a registered suite (a
    /// misconfiguration surfaced as a panic at enumeration time).
    fn from_feature_path(feature_path: &Path) -> Self {
        let parent = feature_path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or_else(|| {
                panic!("feature path has no parent dir: {}", feature_path.display())
            });
        Self::ALL
            .iter()
            .copied()
            .find(|k| k.feature_subdir() == parent)
            .unwrap_or_else(|| panic!("no registered suite for feature dir {parent:?}"))
    }

    /// Runs a single pre-parsed feature through this suite's World.
    ///
    /// Cucumber's `Runner` is generic over the World type, so the two arms
    /// can't share a builder — each must fully configure and launch its own
    /// `::cucumber()` call. The parser + `fail_on_skipped` configuration is
    /// identical across suites.
    async fn run_scenario(self, feature: gherkin::Feature) {
        match self {
            Self::Judge => {
                JudgeWorld::cucumber::<&str>()
                    .with_parser(SingleFeatureParser {
                        feature: Arc::new(feature),
                    })
                    .fail_on_skipped()
                    .run_and_exit("ignored-by-single-feature-parser")
                    .await;
            }
            Self::GapAnalysis => {
                GapAnalysisWorld::cucumber::<&str>()
                    .with_parser(SingleFeatureParser {
                        feature: Arc::new(feature),
                    })
                    .fail_on_skipped()
                    .run_and_exit("ignored-by-single-feature-parser")
                    .await;
            }
        }
    }
}

// ─── Parent: subprocess orchestrator ──────────────────────────────────────

/// Parent mode: enumerate scenarios from every registered suite, spawn one
/// child per scenario, report PASS/FAIL, exit non-zero if any failed.
async fn run_parent() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR").to_owned();
    let scenarios = enumerate_scenarios(&manifest_dir);

    if scenarios.is_empty() {
        eprintln!("[e2e] no scenarios found under tests/features/");
        std::process::exit(1);
    }

    let current_exe = std::env::current_exe().expect("current_exe for child spawning");

    // Cap concurrency conservatively. Each child process boots a full actor
    // system that spawns multiple `spawn_blocking` tasks (SQLite, tokenizer,
    // file scans). At high concurrency the tokio blocking-thread pool
    // saturates and startup actors exceed the tests' bounded wait deadlines,
    // producing non-deterministic failures unrelated to the code under test.
    // Empirically the cliff is at ~2/3 of available parallelism; we cap at
    // half to leave headroom for blocking-thread demand. Override with
    // JINN_E2E_JOBS for debugging.
    let jobs = std::env::var("JINN_E2E_JOBS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|n| (n.get() / 2).max(1))
                .unwrap_or(4)
        });

    let total = scenarios.len();
    eprintln!("[e2e] running {total} scenario(s) across up to {jobs} child process(es)");

    let results: Vec<(ScenarioSpec, bool)> = stream::iter(scenarios.into_iter().map(|spec| {
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
        eprintln!(
            "[e2e] {status}: {}:{}",
            spec.feature_file_name(),
            spec.scenario
        );
        if !passed {
            failures += 1;
        }
    }

    if failures > 0 {
        eprintln!("[e2e] {failures}/{total} scenario(s) FAILED");
        std::process::exit(1);
    }
    eprintln!("[e2e] {total}/{total} scenario(s) passed");
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
            eprintln!("[e2e] failed to spawn child for {spec_str}: {e}");
            false
        }
    }
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

/// Walks every registered suite's feature directory for `*.feature` files and
/// flattens their scenarios into a list. A scenario is identified by
/// `(feature_path, scenario_name)`; its [`WorldKind`] is derived from the
/// feature file's parent directory at child-process time (see [`WorldKind`]).
fn enumerate_scenarios(manifest_dir: &str) -> Vec<ScenarioSpec> {
    let mut out = Vec::new();
    for kind in WorldKind::ALL {
        // A registered suite with no feature dir yet is silently skipped —
        // it has no scenarios to run.
        if let Some(dir) = kind.feature_dir(manifest_dir) {
            enumerate_dir(&dir, &mut out);
        }
    }
    // Stable order: by file then scenario name.
    out.sort_by(|a, b| {
        a.feature_path
            .cmp(&b.feature_path)
            .then_with(|| a.scenario.cmp(&b.scenario))
    });
    out
}

/// Walks a single feature `dir`, appending every scenario in every `.feature`
/// file to `out`.
fn enumerate_dir(dir: &Path, out: &mut Vec<ScenarioSpec>) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("failed to read feature dir {}: {e}", dir.display()));
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
    tracing::info!(spec = %spec, "e2e child starting");
    let (feature_path, scenario_name) = spec.split_once(':').unwrap_or_else(|| {
        panic!("invalid --scenario spec: {spec:?} (expected '<feature_path>:<scenario_name>')")
    });

    eprintln!(
        "[e2e child] cwd={:?} feature_path={:?}",
        std::env::current_dir(),
        feature_path
    );
    let feature = gherkin::Feature::parse_path(feature_path, GherkinEnv::default())
        .unwrap_or_else(|e| panic!("failed to parse {feature_path}: {e}"));

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

    // Route to the suite's World by feature file location, then run cucumber
    // with the filtering parser + default runner/writer.
    // `fail_on_skipped` makes a scenario with no matching steps fail (instead of
    // silently passing), which catches typos in the feature or step patterns.
    let kind = WorldKind::from_feature_path(std::path::Path::new(feature_path));
    kind.run_scenario(single_feature).await;
}

/// Parser that emits exactly one pre-parsed feature (used in child mode).
#[derive(Clone, Debug)]
struct SingleFeatureParser {
    feature: Arc<gherkin::Feature>,
}

impl<I> Parser<I> for SingleFeatureParser {
    type Cli = cucumber::cli::Empty;
    type Output = stream::Once<std::future::Ready<parser::Result<gherkin::Feature>>>;

    fn parse(self, _input: I, _cli: Self::Cli) -> Self::Output {
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
    let log_path = std::env::temp_dir().join(format!("jinn-e2e-{}.log", std::process::id()));
    eprintln!("[e2e] tracing to {}", log_path.display());
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
                    "warn,jinn=debug,jinn_domain=debug,e2e=debug,kameo=warn,kameo_actors=off",
                )
            }),
        )
        .with_target(true)
        .with_writer(file)
        .try_init();
}
