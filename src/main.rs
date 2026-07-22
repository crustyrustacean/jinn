//! Binary entry point for jinn.

use clap::Parser;
use jinn::tracing::{TracingMode, init as init_tracing};
use jinn_cli::Cli;

fn main() {
    // Load .env if present. Not fatal if missing.
    let _ = dotenvy::dotenv();

    let cli = Cli::parse();

    let paths = jinn_domain::AppPaths::default();
    let log_path = cli.log_file.clone().unwrap_or_else(|| paths.log_path());

    let mode = match &cli.command {
        None
        | Some(jinn_cli::cli::Commands::Tui)
        | Some(jinn_cli::cli::Commands::Completions { .. }) => TracingMode::Tui { log_path },
        Some(jinn_cli::cli::Commands::Fetch { .. })
        | Some(jinn_cli::cli::Commands::Config { .. })
        | Some(jinn_cli::cli::Commands::Install) => TracingMode::Headless { log_path },
        #[cfg(debug_assertions)]
        Some(jinn_cli::cli::Commands::Headless { .. }) => TracingMode::Headless { log_path },
    };

    if let Err(e) = init_tracing(cli.verbosity, mode) {
        eprintln!("error: {e:?}");
        std::process::exit(1);
    }

    let mut app = match jinn::App::new() {
        Ok(app) => app,
        Err(e) => {
            eprintln!("error: {e:?}");
            std::process::exit(1);
        }
    };

    if let Err(e) = app.dispatch(cli) {
        eprintln!("error: {e:?}");
        std::process::exit(1);
    }
}
