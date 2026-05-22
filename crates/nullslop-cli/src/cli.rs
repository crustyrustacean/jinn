//! Command-line interface argument definitions.

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use clap_verbosity_flag::{Verbosity, WarnLevel};

/// nullslop — a TUI agent harness with a component/actor system.
#[derive(Debug, Parser)]
#[command(name = "nullslop", version, about)]
pub struct Cli {
    /// Verbosity level for logging.
    #[command(flatten)]
    pub verbosity: Verbosity<WarnLevel>,

    /// Directory for log file output (TUI mode). Defaults to current directory.
    #[arg(long)]
    pub log_dir: Option<PathBuf>,

    /// Session database directory. Defaults to the platform data directory.
    /// Use this to inspect a bench database after a run, e.g.
    /// `nullslop --db-path ./bench.db`.
    #[arg(long)]
    pub db_path: Option<PathBuf>,

    /// The subcommand to run. If omitted, launches the TUI.
    #[command(subcommand)]
    pub command: Option<Commands>,
}

/// Available subcommands.
#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Launch the TUI (default when no subcommand is given).
    Tui,

    /// Run without a terminal interface.
    Headless {
        /// Also log to a file in headless mode.
        #[arg(long)]
        log_file: Option<PathBuf>,

        /// Headless subcommand.
        #[command(subcommand)]
        command: Option<HeadlessCommands>,
    },

    /// Generate shell completions.
    Completions {
        /// The shell to generate completions for.
        shell: clap_complete::Shell,
    },

    /// Run benchmark tasks and view results.
    Bench {
        /// The bench subcommand to run.
        #[command(subcommand)]
        subcommand: BenchCommands,
    },
}

/// Headless subcommands.
#[derive(Debug, Subcommand)]
pub enum HeadlessCommands {
    /// Send a chat message.
    SendChat {
        /// The message text to send.
        message: String,
    },
    /// Run a keystroke script.
    Script {
        /// Path to a script file with one key sequence per line.
        path: String,
    },
}

/// Bench subcommands.
#[derive(Debug, Subcommand)]
pub enum BenchCommands {
    /// Run benchmark tasks through the actor pipeline.
    Run {
        /// Database directory for bench sessions (isolated from user's real database).
        db_path: PathBuf,

        /// Model(s) to benchmark (e.g., `openai/gpt-4o`). At least one required.
        #[arg(long, required = true)]
        model: Vec<String>,

        /// Task(s) to run (e.g., `hello-world`). If omitted, runs all tasks.
        #[arg(long)]
        task: Vec<String>,

        /// Run without TUI.
        #[arg(long)]
        headless: bool,

        /// CSV output path.
        #[arg(long, default_value = "bench-results.csv")]
        csv: PathBuf,

        /// Directory for bench work artifacts (task working directories).
        /// If set, task directories are created here instead of /tmp,
        /// making them easy to inspect after a run.
        #[arg(long)]
        artifact_dir: Option<PathBuf>,
    },

    /// Display bench results in a terminal table.
    Show {
        /// CSV file to display.
        csv: PathBuf,
    },

    /// Compare two bench result CSVs.
    Compare {
        /// First CSV file (baseline).
        csv_a: PathBuf,
        /// Second CSV file (comparison).
        csv_b: PathBuf,
    },

    /// Launch the TUI pointed at a bench database for inspection.
    Tui {
        /// Database file to open.
        db_path: PathBuf,
    },
}
