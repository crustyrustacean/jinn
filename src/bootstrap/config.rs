//! Config reading and service construction for a launch.
//!
//! Everything a run needs before any actor is spawned: the early-return
//! CLI subcommands, the resolved config path, the configuration layer and
//! its validation, the session store, and the services built over them.
//!
//! The order here is behavioural. The recovery subcommands run *first* so
//! `jinn config init` still works when the file it would repair is
//! unreadable; the session store opens *after* they return, so a recovery
//! command never pays for a database open; and validation runs *before*
//! any wiring, so a malformed `jinn.toml` fails fast instead of booting an
//! app that reads defaults.

use std::sync::Arc;

use error_stack::Report;
use error_stack::ResultExt;

use jinn_kernel::ApiKeys;
use jinn_kernel::ApiKeysService;
use jinn_kernel::ConfigStorageService;
use jinn_kernel::FilesystemConfigStorage;
use jinn_kernel::LlmServiceFactoryService;
use jinn_kernel::NoProvidersAvailableFactory;
use jinn_kernel::ProviderRegistry;
use jinn_kernel::ProviderRegistryService;
use jinn_preferences_config::AppStateStorageService;
use jinn_preferences_config::FilesystemAppStateStorage;
use jinn_session_state::SessionStoreService;

use crate::app::AppError;
use crate::app::seed_config_template;
use crate::config_path::config_init_target;
use crate::config_path::resolve_config_path;

/// The services a run is constructed from, before any actor exists.
///
/// Returned whole so the composition root receives a finished set rather
/// than assembling services piecemeal at the point of use.
pub struct LaunchServices {
    /// Backend over `providers.toml`.
    pub config_storage: ConfigStorageService,
    /// Resolved by the env-init actor at boot.
    pub api_keys: ApiKeysService,
    /// Populated by the provider-init actor at boot.
    pub provider_registry: ProviderRegistryService,
    /// The no-provider sentinel until the actors resolve the real one.
    pub llm_service: LlmServiceFactoryService,
    /// The session database.
    pub session_store: SessionStoreService,
    /// The pool behind `session_store`, for headless callers.
    pub session_pool: jinn_discord::backend::spawn::SessionPool,
    /// The validated configuration layer.
    pub config: jinn_config::ConfigLayer,
    /// Persisted app state, reloaded from disk.
    pub app_state_storage: AppStateStorageService,
    /// The shells jinn runs commands through, resolved at launch.
    pub shells: jinn_common::shell::Shells,
}

/// Outcome of the pre-wiring phase.
pub enum Prepared {
    /// A recovery subcommand already ran and the process should exit.
    Handled,
    /// A bundled MCP server should run over stdio with this database
    /// directory, then the process should exit. Carries the `--db` override
    /// so `dispatch` never re-reads the CLI.
    ServeBundled {
        /// Database directory override; `None` uses the jinn-managed default.
        db: Option<std::path::PathBuf>,
        /// Embedding-model override; `None` uses the catalog default.
        embed_model: Option<jinn_mcp_overgraph::embedder::EmbedderModel>,
        /// Warm the embedding model's cache and exit without serving.
        fetch_model: bool,
    },
    /// The run is configured; carry on to wire the actors.
    Ready(LaunchServices),
}

/// Reads config and builds services, or runs a recovery subcommand.
///
/// # Errors
///
/// Returns an error if a recovery subcommand fails, or if the session
/// store cannot be opened. A malformed or invalid `jinn.toml` is *not*
/// an error return: those exit the process with a message, because the
/// user has no actor to see a `Report` from yet.
pub fn prepare(
    cli: &jinn_cli::cli::Cli,
    runtime: &tokio::runtime::Runtime,
) -> Result<Prepared, Report<AppError>> {
    if let Some(handled) = run_recovery_subcommand(cli)? {
        return Ok(handled);
    }

    let (config_storage, config, app_state_storage) = read_config(cli)?;
    let shells = resolve_shells(&config);
    let (session_store, session_pool) = open_session_store(cli, runtime)?;

    Ok(Prepared::Ready(LaunchServices {
        config_storage,
        api_keys: ApiKeysService::new(ApiKeys::new()),
        provider_registry: ProviderRegistryService::new(
            ProviderRegistry::from_config(jinn_kernel::ProvidersConfig {
                providers: std::collections::BTreeMap::new(),
                aliases: vec![],
                default_provider: None,
                endpoint_defaults: vec![],
            })
            .change_context(AppError)?,
        ),
        llm_service: LlmServiceFactoryService::new(Arc::new(NoProvidersAvailableFactory)),
        session_store,
        session_pool,
        config,
        app_state_storage,
        shells,
    }))
}

/// Resolves the shells jinn runs commands through, from the `[shell]`
/// section of `jinn.toml`.
///
/// A configured path that names no file is a typo, and it stops the
/// launch: the user configured that executable deliberately, and falling
/// through to auto-detection would silently run something else — on
/// Windows, potentially the WSL launcher. A shell that is simply not
/// installed is a fact about the machine, not a configuration error, so
/// it is reported at the point of use instead.
///
/// The environment is probed here, at launch, and never again: the
/// resolved value is carried on [`jinn_kernel::Services`] from here on.
fn resolve_shells(config: &jinn_config::ConfigLayer) -> jinn_common::shell::Shells {
    // A malformed `[shell]` section is a broken file, not a broken
    // machine: it stops the launch the same way a broken `jinn.toml`
    // does, naming the section.
    let shell_config = match config.get::<jinn_preferences_config::schemas::ShellConfig>() {
        Ok(shell_config) => shell_config,
        Err(error) => {
            eprintln!("error: [shell] in jinn.toml is malformed: {error}");
            std::process::exit(1);
        }
    };
    let shells = jinn_common::shell::Shells::resolve(
        shell_config.bash_path.as_deref(),
        shell_config.powershell_path.as_deref(),
    );
    for error in [shells.bash().err(), shells.powershell().err()]
        .into_iter()
        .flatten()
    {
        if error.is_configuration_error() {
            eprintln!("error: {error}");
            std::process::exit(1);
        }
    }
    shells
}

/// The subcommands that must run before anything is parsed or opened.
///
/// `jinn config init` is the recovery tool for a missing or broken config,
/// so it must not be guarded by the parsing that auto-creates the file on
/// first run — and it needs no session store.
fn run_recovery_subcommand(cli: &jinn_cli::cli::Cli) -> Result<Option<Prepared>, Report<AppError>> {
    use jinn_cli::cli::Commands;

    if let Some(Commands::Config { subcommand }) = &cli.command {
        use jinn_cli::cli::ConfigCommands;
        use jinn_preferences_config::{
            InitOutcome, StreamRulesSeedOutcome, init_default_config_to, init_stream_rules_to,
            preferences_path,
        };

        match subcommand {
            ConfigCommands::Init { force } => {
                // Honor --config here: the user naming a path is asking
                // for the file to land there. Deliberately skips the
                // existence check the runtime resolver applies, since
                // creating the file is what this command is for.
                let path = config_init_target(cli.config.as_deref(), &preferences_path());
                let force = *force;
                match init_default_config_to(&path, force) {
                    Ok(InitOutcome::Created) => {
                        println!("Created {}", path.display());
                    }
                    Ok(InitOutcome::Overwritten) => {
                        println!("Overwrote {}", path.display());
                    }
                    Err(report) => {
                        eprintln!("{report:?}");
                        return Err(report.change_context(AppError));
                    }
                }

                // The stream-rules file lives beside `jinn.toml`, wherever
                // this command just put that document. `--force` overwrites
                // it too — that is the supported path for pulling in an
                // updated shipped set. Without `--force` an existing file is
                // left alone and reported as skipped, not failed: the
                // command's primary target (`jinn.toml`) already succeeded.
                let rules_path = path
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new("."))
                    .join(jinn_preferences_config::STREAM_RULES_FILE_NAME);
                match init_stream_rules_to(&rules_path, force) {
                    Ok(StreamRulesSeedOutcome::Created(p)) => {
                        println!("Created {}", p.display());
                    }
                    Ok(StreamRulesSeedOutcome::Overwritten(p)) => {
                        println!("Overwrote {}", p.display());
                    }
                    Ok(StreamRulesSeedOutcome::Untouched(p)) => {
                        println!("Already present, skipped {}", p.display());
                    }
                    Err(report) => {
                        eprintln!("{report:?}");
                        return Err(report.change_context(AppError));
                    }
                }
                return Ok(Some(Prepared::Handled));
            }
            ConfigCommands::Providers { force } => {
                use jinn_kernel::{InitProvidersOutcome, config_path, init_default_providers_to};

                let path = config_path();
                let force = *force;
                match init_default_providers_to(&path, force) {
                    Ok(InitProvidersOutcome::Created) => {
                        println!("Created {}", path.display());
                    }
                    Ok(InitProvidersOutcome::Overwritten) => {
                        println!("Overwrote {}", path.display());
                    }
                    Err(report) => {
                        eprintln!("{report:?}");
                        return Err(report.change_context(AppError));
                    }
                }
                return Ok(Some(Prepared::Handled));
            }
        }
    }

    // `install` seeds default resources into user dirs. Like `config`, it
    // must run before any actor wiring — and it needs no preferences/DB,
    // so it dispatches before the session store is opened.
    if let Some(Commands::Install { force }) = &cli.command {
        use jinn_install::{
            Destinations, InstallOutcome, InstallReport, JinnTomlOutcome, install_defaults_to,
        };
        use jinn_kernel::AppPaths;
        use jinn_preferences_config::StreamRulesSeedOutcome;

        let app_paths = AppPaths::default();
        let config_path = jinn_config::FilesystemConfigStorage::default_path()
            .path()
            .to_path_buf();
        let destinations = Destinations::new(
            app_paths.themes_dir(),
            app_paths.personas_dir(),
            app_paths.prompts_dir(),
            app_paths.workflows_dir(),
            app_paths.skills_dir(),
        );
        match install_defaults_to(&destinations, *force, &config_path) {
            Ok(report) => {
                let InstallReport {
                    outcomes,
                    jinn_toml,
                    stream_rules,
                } = report;
                for outcome in outcomes {
                    match &outcome {
                        InstallOutcome::Created(path) => {
                            println!("Installed {}", path.display());
                        }
                        InstallOutcome::Skipped(path) => {
                            println!("Already present, skipped {}", path.display());
                        }
                        InstallOutcome::Overwritten(path) => {
                            println!("Overwrote {}", path.display());
                        }
                    }
                }
                match &jinn_toml {
                    JinnTomlOutcome::Created(path) => {
                        println!("Created {}", path.display());
                    }
                    JinnTomlOutcome::Untouched(path) => {
                        println!("Already present, skipped {}", path.display());
                    }
                }
                match &stream_rules {
                    StreamRulesSeedOutcome::Created(path) => {
                        println!("Created {}", path.display());
                    }
                    // Install never modifies an existing rules file, so
                    // Untouched is the only other reachable variant here.
                    StreamRulesSeedOutcome::Untouched(path)
                    | StreamRulesSeedOutcome::Overwritten(path) => {
                        println!("Already present, skipped {}", path.display());
                    }
                }
                return Ok(Some(Prepared::Handled));
            }
            Err(report) => {
                eprintln!("error: failed to install defaults:");
                eprintln!("  {report:?}");
                return Err(report.change_context(AppError));
            }
        }
    }

    // The bundled MCP servers (`jinn mcp overgraph`) are child-process
    // programs: they speak stdio MCP and touch nothing from the parent's
    // environment — no jinn.toml, no providers, no sessions.db. They
    // intercept here so a server spawn never opens the session store or
    // mutates the user's config as a side effect; `dispatch` runs the
    // server from the returned variant.
    if let Some(Commands::Mcp {
        subcommand:
            jinn_cli::cli::McpCommands::Overgraph {
                db,
                embed_model,
                fetch_model,
            },
    }) = &cli.command
    {
        // A malformed --embed-model is a CLI-level usage error: it is
        // rendered here (before any transport exists) and exits the process,
        // matching how a bad flag itself would fail. The server never starts.
        let embed_model = match embed_model {
            Some(name) => match jinn_mcp_overgraph::embedder::EmbedderModel::from_arg(name) {
                Some(model) => Some(model),
                None => {
                    #[expect(
                        clippy::print_stderr,
                        reason = "fatal CLI usage failure before any transport starts"
                    )]
                    let () = {
                        eprintln!(
                            "error: unknown embedding model `{name}`; \
                                 valid models: bge-small-en-v1.5"
                        );
                    };
                    return Err(
                        Report::new(AppError).attach(format!("unknown --embed-model `{name}`"))
                    );
                }
            },
            None => None,
        };
        return Ok(Some(Prepared::ServeBundled {
            db: db.clone(),
            embed_model,
            fetch_model: *fetch_model,
        }));
    }

    Ok(None)
}

/// Checks that `providers.toml` loads and parses, producing a fail-fast report.
///
/// A missing file is not an error here — the loader auto-creates the default
/// template on first run. Only a load or parse failure produces an error, with
/// the config path and the underlying TOML detail attached to the report.
fn providers_load_error_report(storage: &ConfigStorageService) -> Result<(), Report<AppError>> {
    if let Err(report) = storage.load() {
        let path = jinn_kernel::config_path();
        return Err(report.change_context(AppError).attach(format!(
            "failed to load providers config at {}",
            path.display()
        )));
    }
    Ok(())
}

/// Resolves the config path, loads the layer, validates it, and reloads
/// app state.
type ConfigTriple = (
    ConfigStorageService,
    jinn_config::ConfigLayer,
    AppStateStorageService,
);

fn read_config(cli: &jinn_cli::cli::Cli) -> Result<ConfigTriple, Report<AppError>> {
    let config_storage =
        ConfigStorageService::new(Arc::new(FilesystemConfigStorage::default_path()));

    // Resolve which jinn.toml this run reads and writes, and seed the
    // template when the default location is still missing.
    //
    // This runs AFTER the recovery subcommands above so neither gets
    // pre-seeded ahead of itself, and BEFORE the layer load so the seeded
    // file is what the layer reads.
    let resolved_config = resolve_config_path(
        cli.config.as_deref(),
        &jinn_preferences_config::preferences_path(),
    );
    let config_path = match resolved_config {
        Ok(resolved) => {
            if resolved.seed_template {
                seed_config_template(&resolved.path);
            }
            resolved.path
        }
        Err(report) => {
            eprintln!("error: failed to resolve the configuration path:");
            eprintln!("  {report:?}");
            std::process::exit(1);
        }
    };

    // The layer is the only reader of this document; there is no separate
    // aggregate struct to parse alongside it. The storage is built over
    // the RESOLVED path, so `--config` redirects reads and writes alike.
    let config = {
        let backend = jinn_config::FilesystemConfigStorage::new(config_path.clone());
        match jinn_config::ConfigLayer::load(Arc::new(backend)) {
            Ok(layer) => layer,
            Err(report) => {
                tracing::error!(path = %config_path.display(), "failed to load the jinn.toml configuration layer");
                eprintln!("error: failed to parse {}:", config_path.display());
                eprintln!("  {report:?}");
                std::process::exit(1);
            }
        }
    };

    // Fail-fast on a malformed section before any actor wiring runs.
    // `validate` only walks sections registered on the layer, so the
    // roster goes in first — an unregistered section is never checked
    // and a malformed table would boot to a running app reading
    // defaults. Registration must precede the check, and both must
    // precede the composition root's boot.
    jinn_preferences_config::register_all_sections(&config);

    if let Err(error) = config.validate() {
        tracing::error!(%error, "jinn.toml section failed validation");
        eprintln!("error: {error}");
        std::process::exit(1);
    }

    // Fail-fast on a malformed providers.toml before any actor wiring
    // runs. `jinn config providers` has already dispatched, so it
    // remains usable as the recovery tool for a broken file.
    if let Err(report) = providers_load_error_report(&config_storage) {
        tracing::error!("failed to load providers config");
        eprintln!("error: failed to load providers config:");
        eprintln!("  {report:?}");
        std::process::exit(1);
    }

    let app_state_storage = {
        let backend =
            FilesystemAppStateStorage::new(jinn_kernel::AppPaths::default().state_file_path());
        let svc = AppStateStorageService::new(Arc::new(backend));
        if let Err(report) = svc.reload() {
            tracing::error!("failed to load app state");
            eprintln!("error: failed to load app state:");
            eprintln!("  {report:?}");
            std::process::exit(1);
        }
        svc
    };

    Ok((config_storage, config, app_state_storage))
}

/// Opens the session database, honouring `--db-path`.
fn open_session_store(
    cli: &jinn_cli::cli::Cli,
    runtime: &tokio::runtime::Runtime,
) -> Result<
    (
        SessionStoreService,
        jinn_discord::backend::spawn::SessionPool,
    ),
    Report<AppError>,
> {
    let store = runtime.block_on(async {
        match cli.db_path_opt() {
            Some(path) => {
                jinn_session_store::sqlite::SqliteSessionStore::open_or_create(path).await
            }
            None => jinn_session_store::sqlite::SqliteSessionStore::new().await,
        }
    });
    let store = store.change_context(AppError)?;
    let pool = store.pool().clone();
    Ok((SessionStoreService::new(Arc::new(store)), pool))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, reason = "test code")]

    use jinn_kernel::ConfigStorageService;
    use jinn_kernel::FilesystemConfigStorage;
    use std::sync::Arc;

    use super::providers_load_error_report;
    #[rstest::rstest]
    fn providers_load_error_report_fails_with_parse_detail_on_malformed_file() {
        // Given a config storage backed by a malformed providers.toml.
        let dir = jinn_testutil::tempdir().expect("temp dir");
        let path = dir.path().join("providers.toml");
        std::fs::write(
            &path,
            "[providers.ollama]\nbackend = \"ollama\"\nmodels = [\"llama3\"\n",
        )
        .expect("write");
        let storage = ConfigStorageService::new(Arc::new(FilesystemConfigStorage::new(path)));

        // When checking the providers config.
        let result = providers_load_error_report(&storage);

        // Then the error render keeps the TOML detail attached upstream
        // (attachments survive the change_context to AppError).
        let report = result.expect_err("malformed providers.toml must fail");
        let rendered = format!("{report:?}");
        assert!(
            rendered.contains("TOML parse error"),
            "missing TOML parse detail: {rendered}"
        );
    }

    #[rstest::rstest]
    fn providers_load_error_report_ok_when_file_missing() {
        // Given a config storage backed by a directory with no providers.toml.
        let dir = jinn_testutil::tempdir().expect("temp dir");
        let path = dir.path().join("providers.toml");
        let storage =
            ConfigStorageService::new(Arc::new(FilesystemConfigStorage::new(path.clone())));

        // When checking the providers config.
        let result = providers_load_error_report(&storage);

        // Then the check passes (the loader auto-creates the default template).
        assert!(result.is_ok(), "expected ok, got: {:?}", result.err());
        // And the default file was created.
        assert!(
            path.exists(),
            "first-run load should auto-create the config file"
        );
    }
}
