//! Binary entry point for jinn.

use clap::Parser;
use jinn::tracing::{TracingMode, init as init_tracing};
use jinn_cli::Cli;

fn main() {
    // Load .env if present. Not fatal if missing.
    let _ = dotenvy::dotenv();

    let cli = Cli::parse();

    let mode = match &cli.command {
        None | Some(jinn_cli::cli::Commands::Tui) => TracingMode::Tui {
            log_dir: cli.log_dir.clone(),
        },
        Some(jinn_cli::cli::Commands::Headless { log_file, .. }) => TracingMode::Headless {
            log_file: log_file.clone(),
        },
        Some(jinn_cli::cli::Commands::Completions { .. }) => TracingMode::Tui { log_dir: None },
        Some(jinn_cli::cli::Commands::Bench { .. }) => TracingMode::Tui { log_dir: None },
        Some(jinn_cli::cli::Commands::Fetch { .. }) => TracingMode::Headless { log_file: None },
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
