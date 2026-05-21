//! Task runner — the main orchestration loop for bench execution.
//!
//! Validates models, iterates task×model pairs, sends messages through the
//! actor system, captures stats, and writes CSV progressively.

#![allow(
    clippy::print_stdout,
    reason = "bench progress output goes to stdout"
)]
#![allow(
    clippy::print_stderr,
    reason = "bench error output goes to stderr"
)]
#![allow(
    clippy::exit,
    reason = "bench runner uses exit for fatal errors"
)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nullslop_domain::feat::provider_infra::{
    ApiKeys, ApiKeysService, ConfigStorageService, FilesystemConfigStorage,
    LlmServiceFactoryService, NoProvidersAvailableFactory,
    ProviderId, ProviderRegistry, ProviderRegistryService, ProvidersConfig,
};
use nullslop_domain::feat::session::token_stats::TokenStats;
use nullslop_domain::feat::session::{SessionStoreService, SqliteSessionStore};
use nullslop_domain::{
    AppCore, ChatEntry, Command, EnqueueUserMessage,
    InMemoryUserPreferencesStorage, ProviderSwitch, SessionPhase, State,
    UserPreferencesStorageService,
};

use crate::cli;
use crate::csv::{BenchCsvWriter, BenchResult};
use crate::fixture;
use crate::task::BenchTask;
use crate::tasks;
use crate::wiring::{self, BenchWiringConfig};

/// Status of a single task/model run.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RunStatus {
    Completed,
    Timeout,
}

/// Runs the full bench suite.
///
/// # Errors
///
/// Returns an error if model validation fails or the CSV cannot be created.
pub fn run_bench(
    handle: &tokio::runtime::Handle,
    args: &cli::RunArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    let all_tasks = tasks::bench_tasks();
    let filtered = filter_tasks(all_tasks, &args.only, &args.exclude);

    if filtered.is_empty() {
        eprintln!("error: no tasks matched the filter");
        std::process::exit(1);
    }

    // Create service dependencies (shared across all runs).
    let services = create_shared_services(args)?;

    // Validate models against registry.
    if let Err(invalid) = validate_models(&args.models, &services.provider_registry) {
        return Err(format!("unknown models: {}", invalid.join(", ")).into());
    }

    // Create output directory.
    let timestamp = jiff::Timestamp::now()
        .strftime("%Y-%m-%dT%H-%M-%S")
        .to_string();
    let output_dir = PathBuf::from("bench-results").join(&timestamp);
    std::fs::create_dir_all(&output_dir)?;

    let csv_path = output_dir.join("results.csv");
    let mut csv_writer = BenchCsvWriter::create(&csv_path)?;

    // Set up Ctrl+C handler — CSV is flushed after every row, so partial results survive.
    ctrlc::set_handler(|| {
        eprintln!("\ninterrupted — partial results saved");
        std::process::exit(0);
    })?;

    let total = filtered.len() * args.models.len();
    let mut idx = 0;

    for task in &filtered {
        for model in &args.models {
            idx += 1;
            print!(
                "[{idx}/{total}] {} | {} ... ",
                task.name, model
            );
            let start = Instant::now();

            let result = run_single_task(handle, task, model, &output_dir, &services);

            let elapsed = start.elapsed();
            let wall_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);

            let (status, passed) = match result.status {
                RunStatus::Completed => {
                    let p = (task.verify)(&result.work_dir);
                    ("completed".to_owned(), p)
                }
                RunStatus::Timeout => ("timeout".to_owned(), false),
            };

            let bench_result = BenchResult {
                name: task.name.to_owned(),
                model: model.clone(),
                turns: result.turns,
                tokens_in: result.tokens_in,
                tokens_out: result.tokens_out,
                cost: result.cost,
                wall_time_ms: wall_ms,
                passed,
                status,
            };

            csv_writer.write_row(&bench_result)?;

            let check = if bench_result.passed { "✓" } else { "✗" };
            let sec = elapsed.as_secs_f64();
            println!(
                "{} ({sec:.1}s, ↑{} ↓{} tokens, {check})",
                bench_result.status,
                bench_result.tokens_in, bench_result.tokens_out,
            );
        }
    }

    println!("\nresults written to {}", csv_path.display());
    Ok(())
}

/// Shared service instances created once for the entire bench run.
struct SharedServices {
    provider_registry: ProviderRegistryService,
    config_storage: ConfigStorageService,
    api_keys: ApiKeysService,
}

/// Creates service instances shared across all task/model runs.
fn create_shared_services(_args: &cli::RunArgs) -> Result<SharedServices, Box<dyn std::error::Error>> {
    // Provider registry starts empty — provider-init actor populates it from providers.toml.
    let empty_config = ProvidersConfig {
        providers: vec![],
        aliases: vec![],
        default_provider: None,
    };
    let provider_registry = ProviderRegistryService::new(ProviderRegistry::from_config(empty_config)?);

    // Config storage reads providers.toml.
    let config_storage = ConfigStorageService::new(Arc::new(FilesystemConfigStorage::default_path()));

    // API keys are resolved by the env-init actor.
    let api_keys = ApiKeysService::new(ApiKeys::new());

    Ok(SharedServices {
        provider_registry,
        config_storage,
        api_keys,
    })
}

/// Result of running a single task/model pair.
struct TaskRunResult {
    work_dir: PathBuf,
    turns: u32,
    tokens_in: u64,
    tokens_out: u64,
    cost: f64,
    status: RunStatus,
}

/// Runs a single task against a single model.
fn run_single_task(
    handle: &tokio::runtime::Handle,
    task: &BenchTask,
    model: &str,
    output_dir: &std::path::Path,
    shared: &SharedServices,
) -> TaskRunResult {
    let work_dir = output_dir.join(task.name).join(model.replace('/', "_"));

    // Prepare fixture.
    fixture::prepare_fixture(task.fixture_dir, &work_dir)
        .unwrap_or_else(|e| {
            eprintln!("error preparing fixture: {e}");
            std::process::exit(1);
        });

    // Create a bench root directory for database isolation.
    let bench_root = work_dir.join(".bench-data");
    std::fs::create_dir_all(&bench_root).unwrap_or_else(|e| {
        eprintln!("error creating bench root: {e}");
        std::process::exit(1);
    });

    let session_store = SessionStoreService::new(Arc::new(
        SqliteSessionStore::new_in(&bench_root).unwrap_or_else(|e| {
            eprintln!("error creating session store: {e}");
            std::process::exit(1);
        }),
    ));

    let llm_service = LlmServiceFactoryService::new(Arc::new(NoProvidersAvailableFactory));

    let builtin_tools: Vec<String> = task
        .tools
        .builtins
        .iter()
        .map(|s| (*s).to_owned())
        .collect();

    let config = BenchWiringConfig {
        bench_root,
        llm_service,
        provider_registry: shared.provider_registry.clone(),
        api_keys: shared.api_keys.clone(),
        config_storage: shared.config_storage.clone(),
        session_store,
        user_preferences_storage: UserPreferencesStorageService::new(Arc::new(
            InMemoryUserPreferencesStorage::new(),
        )),
        builtin_tools,
        custom_tools: task.tools.custom.clone(),
        session_cwd: work_dir.clone(),
    };

    let (core, _services, actor_host) = wiring::create_bench_core(handle, config);

    // Switch to the requested model.
    switch_model(&core, model);

    // Set persona if specified.
    if let Some(persona) = task.persona {
        let mut guard = core.state.write();
        guard.active_session_mut().set_persona_name(persona.to_owned());
    }

    let start = Instant::now();
    let mut timed_out = false;

    // Send messages sequentially, waiting for Idle between each.
    for message in &task.messages {
        send_message(&core, message);

        let remaining = task
            .timeout
            .checked_sub(start.elapsed())
            .unwrap_or(Duration::ZERO);

        if matches!(wait_for_idle(&core.state, remaining), WaitOutcome::Timeout) {
            timed_out = true;
            break;
        }
    }

    // Extract stats before shutdown.
    let (turns, tokens_in, tokens_out, cost) = extract_stats(&core.state);

    // Tear down.
    wiring::shutdown_bench(&actor_host, &core.state, handle);

    let status = if timed_out {
        RunStatus::Timeout
    } else {
        RunStatus::Completed
    };

    TaskRunResult {
        work_dir,
        turns,
        tokens_in,
        tokens_out,
        cost,
        status,
    }
}

/// Sends a ProviderSwitch command for the given model.
fn switch_model(core: &AppCore, model: &str) {
    let session_id = {
        let state = core.state.read();
        state.session.active_session_id().clone()
    };
    core.submit_command(Command::ProviderSwitch(ProviderSwitch {
        session_id,
        provider_id: model.to_owned(),
    }));

    // Wait for the model to change (with a generous timeout).
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        {
            let guard = core.state.read();
            let current = guard.active_session().model();
            if current == model {
                return;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    eprintln!("warning: model switch to {model} timed out");
}

/// Sends a user message via EnqueueUserMessage.
fn send_message(core: &AppCore, text: &str) {
    let session_id = {
        let state = core.state.read();
        state.session.active_session_id().clone()
    };
    core.submit_command(Command::EnqueueUserMessage(EnqueueUserMessage {
        session_id,
        entry: ChatEntry::user(text.to_owned()),
    }));
}

/// Result of waiting for session idle.
enum WaitOutcome {
    Completed,
    Timeout,
}

/// Polls the session phase until it reaches Idle or the timeout expires.
fn wait_for_idle(state: &State, timeout: Duration) -> WaitOutcome {
    let deadline = Instant::now() + timeout;
    loop {
        {
            let guard = state.read();
            let phase = guard.active_session().phase();
            if matches!(phase, SessionPhase::Idle) {
                return WaitOutcome::Completed;
            }
        }
        if Instant::now() >= deadline {
            return WaitOutcome::Timeout;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Extracts stats from the session state.
fn extract_stats(state: &State) -> (u32, u64, u64, f64) {
    let guard = state.read();
    let session = guard.active_session();

    let ledger = session.token_ledger();
    let stats = TokenStats::from_ledger(ledger);
    let cost = TokenStats::total_cost(ledger);

    let turns = u32::try_from(
        session
            .history()
            .iter()
            .filter(|e| {
                matches!(
                    e.kind,
                    nullslop_domain::feat::session::chat_entry::ChatEntryKind::User { .. }
                        | nullslop_domain::feat::session::chat_entry::ChatEntryKind::Assistant(..)
                )
            })
            .count(),
    )
    .unwrap_or(u32::MAX);

    (turns, stats.total_sent, stats.total_received, cost)
}

/// Validates that all requested models exist in the provider registry.
///
/// # Errors
///
/// Returns a list of invalid model IDs if any don't exist.
fn validate_models(
    models: &[String],
    registry: &ProviderRegistryService,
) -> Result<(), Vec<String>> {
    let invalid: Vec<String> = models
        .iter()
        .filter(|m| {
            let id = ProviderId::new((*m).clone());
            registry.get(&id).is_none()
        })
        .cloned()
        .collect();
    if invalid.is_empty() {
        Ok(())
    } else {
        Err(invalid)
    }
}

/// Filters tasks based on `--only` and `--exclude` flags.
fn filter_tasks(
    tasks: Vec<BenchTask>,
    only: &Option<String>,
    exclude: &Option<String>,
) -> Vec<BenchTask> {
    let only_set: Option<std::collections::HashSet<String>> = only
        .as_ref()
        .map(|s| s.split(',').map(|n| n.trim().to_owned()).collect());
    let exclude_set: Option<std::collections::HashSet<String>> = exclude
        .as_ref()
        .map(|s| s.split(',').map(|n| n.trim().to_owned()).collect());

    tasks
        .into_iter()
        .filter(|t| {
            if let Some(ref only_names) = only_set
                && !only_names.contains(t.name) {
                    return false;
                }
            if let Some(ref exclude_names) = exclude_set
                && exclude_names.contains(t.name) {
                    return false;
                }
            true
        })
        .collect()
}
