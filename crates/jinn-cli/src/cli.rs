//! Command-line interface argument definitions.

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use clap_verbosity_flag::{Verbosity, WarnLevel};

/// jinn - a TUI agent harness with a component/actor system.
#[derive(Debug, Parser)]
#[command(name = "jinn", version, about)]
pub struct Cli {
    /// Verbosity level for logging.
    #[command(flatten)]
    pub verbosity: Verbosity<WarnLevel>,

    /// Path to the log file. Defaults to the platform's state directory
    /// (e.g. `~/.local/state/jinn/jinn.log` on Linux).
    #[arg(long, value_hint = clap::ValueHint::FilePath)]
    pub log_file: Option<PathBuf>,

    /// Session database file. Defaults to the platform data directory.
    /// Use this to inspect a bench database after a run, e.g.
    /// `jinn --db-path ./bench.db/sessions.db`.
    #[arg(long, value_hint = clap::ValueHint::FilePath)]
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

    /// Fetch reference data from external sources.
    Fetch {
        /// The fetch subcommand to run.
        #[command(subcommand)]
        subcommand: FetchCommands,
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

/// Fetch subcommands.
#[derive(Debug, Subcommand)]
pub enum FetchCommands {
    /// Fetch model metadata from models.dev and save locally.
    Models,
}

/// Bench subcommands.
#[derive(Debug, Subcommand)]
pub enum BenchCommands {
    /// Run benchmark tasks through the actor pipeline.
    Run {
        /// Database file path for bench sessions (isolated from user's real database).
        #[arg(value_hint = clap::ValueHint::FilePath)]
        db_path: PathBuf,

        /// Model(s) to benchmark (e.g., `openai/gpt-4o`). At least one required.
        #[arg(long, required = true)]
        model: Vec<String>,

        /// Task(s) to run. Supports glob patterns (e.g., `edit-*`, `fix-*`).
        /// If omitted, runs all tasks.
        #[arg(long)]
        task: Vec<String>,

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
        #[arg(value_hint = clap::ValueHint::FilePath)]
        db_path: PathBuf,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    // Given no --log-file argument.
    // Then Cli.log_file is None (default).
    #[test]
    fn log_file_flag_defaults_to_none() {
        let cli = Cli::parse_from(["jinn"]);
        assert!(cli.log_file.is_none());
    }

    // Given a global --log-file argument before a subcommand.
    // Then Cli.log_file captures the override.
    #[test]
    fn log_file_flag_global_overrides() {
        let cli = Cli::parse_from(["jinn", "--log-file", "/tmp/x.log", "tui"]);
        assert_eq!(cli.log_file.as_deref(), Some(std::path::Path::new("/tmp/x.log")));
    }

    // Given the old --log-dir argument.
    // Then clap rejects it (the flag has been removed).
    #[test]
    fn log_dir_flag_removed() {
        let result = Cli::try_parse_from(["jinn", "--log-dir", "/tmp"]);
        assert!(result.is_err());
    }

    // Given --log-file scoped to the headless subcommand (old shape).
    // Then clap rejects it: the flag is global now, not a subcommand arg.
    #[test]
    fn headless_scoped_log_file_removed() {
        let result = Cli::try_parse_from([
            "jinn",
            "headless",
            "--log-file",
            "/tmp/x.log",
            "send-chat",
            "hi",
        ]);
        assert!(result.is_err());
    }

    // Given a global --log-file alongside a headless subcommand.
    // Then Cli.log_file captures the override.
    #[test]
    fn log_file_flag_works_with_headless() {
        let cli = Cli::parse_from([
            "jinn", "--log-file", "/tmp/x.log", "headless", "send-chat", "hi",
        ]);
        assert_eq!(cli.log_file.as_deref(), Some(std::path::Path::new("/tmp/x.log")));
    }
}
