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
use crate::app::providers_load_error_report;
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
}

/// Outcome of the pre-wiring phase.
pub enum Prepared {
    /// A recovery subcommand already ran and the process should exit.
    Handled,
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
    let (session_store, session_pool) = open_session_store(cli, runtime)?;

    Ok(Prepared::Ready(LaunchServices {
        config_storage,
        api_keys: ApiKeysService::new(ApiKeys::new()),
        provider_registry: ProviderRegistryService::new(
            ProviderRegistry::from_config(jinn_kernel::ProvidersConfig {
                providers: std::collections::BTreeMap::new(),
                aliases: vec![],
                default_provider: None,
            })
            .change_context(AppError)?,
        ),
        llm_service: LlmServiceFactoryService::new(Arc::new(NoProvidersAvailableFactory)),
        session_store,
        session_pool,
        config,
        app_state_storage,
    }))
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
        use jinn_preferences_config::{InitOutcome, init_default_config_to, preferences_path};

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

        let app_paths = AppPaths::default();
        let config_path = jinn_config::FilesystemConfigStorage::default_path()
            .path()
            .to_path_buf();
        let destinations = Destinations::new(
            app_paths.themes_dir(),
            app_paths.personas_dir(),
            app_paths.prompts_dir(),
            app_paths.skills_dir(),
        );
        match install_defaults_to(&destinations, *force, &config_path) {
            Ok(report) => {
                let InstallReport {
                    outcomes,
                    jinn_toml,
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
                return Ok(Some(Prepared::Handled));
            }
            Err(report) => {
                eprintln!("error: failed to install defaults:");
                eprintln!("  {report:?}");
                return Err(report.change_context(AppError));
            }
        }
    }

    Ok(None)
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
