//! CLI argument definitions for nullslop-bench.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// nullslop-bench — harness benchmarking suite.
#[derive(Debug, Parser)]
#[command(name = "nullslop-bench", version, about)]
pub struct Cli {
    /// The subcommand to run.
    #[command(subcommand)]
    pub command: Commands,
}

/// Arguments for the `run` subcommand.
#[expect(dead_code, reason = "used by runner in future phases")]
pub struct RunArgs {
    /// Model IDs to bench.
    pub models: Vec<String>,
    /// Database path.
    pub db: PathBuf,
    /// Only these tasks.
    pub only: Option<String>,
    /// Exclude these tasks.
    pub exclude: Option<String>,
}

impl RunArgs {
    /// Extracts from the CLI `Run` variant.
    #[must_use]
    pub fn from_run(models: Vec<String>, db: PathBuf, only: Option<String>, exclude: Option<String>) -> Self {
        Self { models, db, only, exclude }
    }
}

/// Available subcommands.
#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Run benchmark tasks against one or more models.
    Run {
        /// Model IDs to bench (e.g., "openai/gpt-4o"). At least one required.
        #[arg(long = "model", required = true)]
        models: Vec<String>,

        /// Database path. Defaults to ./nullslop-bench.sqlite
        #[arg(long, default_value = "nullslop-bench.sqlite")]
        db: PathBuf,

        /// Run only these tasks (comma-separated names).
        #[arg(long)]
        only: Option<String>,

        /// Exclude these tasks (comma-separated names).
        #[arg(long, conflicts_with = "only")]
        exclude: Option<String>,
    },

    /// Render bench results as a terminal table.
    Show {
        /// Path to bench CSV file.
        csv: PathBuf,
    },

    /// Compare two bench runs and show deltas.
    Compare {
        /// Baseline CSV.
        csv_a: PathBuf,
        /// New CSV.
        csv_b: PathBuf,
    },
}
