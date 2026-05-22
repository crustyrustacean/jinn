//! nullslop-bench — harness benchmarking suite.
//!
//! Runs programmatic tasks through the nullslop harness pipeline and captures
//! per-task statistics for comparison across code changes.

#![allow(clippy::exit, reason = "CLI uses exit for fatal errors")]

use clap::Parser;

use nullslop_bench::cli;
use nullslop_bench::compare;
use nullslop_bench::runner;
use nullslop_bench::show;

fn main() {
    let cli = cli::Cli::parse();

    match cli.command {
        cli::Commands::Run {
            models,
            db,
            only,
            exclude,
        } => {
            let args = cli::RunArgs::from_run(models, db, only, exclude);
            let runtime = tokio::runtime::Runtime::new().unwrap_or_else(|e| {
                eprintln!("error: failed to create tokio runtime: {e}");
                std::process::exit(1);
            });
            if let Err(e) = runner::run_bench(runtime.handle(), &args) {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        }
        cli::Commands::Show { csv } => {
            if let Err(e) = show::show_results(&csv) {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        }
        cli::Commands::Compare { csv_a, csv_b } => {
            if let Err(e) = compare::compare_results(&csv_a, &csv_b) {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        }
    }
}
