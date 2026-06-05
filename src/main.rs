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
        | Some(jinn_cli::cli::Commands::Completions { .. })
        | Some(jinn_cli::cli::Commands::Bench { .. }) => TracingMode::Tui {
            log_path: log_path.clone(),
        },
        Some(jinn_cli::cli::Commands::Headless { .. })
        | Some(jinn_cli::cli::Commands::Fetch { .. }) => TracingMode::Headless {
            log_path: log_path.clone(),
        },
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
