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
    ///
    /// In debug builds this flag is **required** to prevent accidental use
    /// of the production database during development.
    #[cfg(debug_assertions)]
    #[arg(long, value_hint = clap::ValueHint::FilePath)]
    pub db_path: PathBuf,

    /// Session database file. Defaults to the platform data directory.
    ///
    /// Use `--db-path` to inspect a bench database after a run, e.g.
    /// `jinn --db-path ./bench.db/sessions.db`.
    #[cfg(not(debug_assertions))]
    #[arg(long, value_hint = clap::ValueHint::FilePath)]
    pub db_path: Option<PathBuf>,

    /// Base directory for persistent browser profiles (headless/headed).
    ///
    /// Per-mode profiles live under `<dir>/headless` and `<dir>/headed`.
    /// Defaults to the platform data directory
    /// (`~/.local/share/jinn/browser-profile`).
    #[arg(long, value_hint = clap::ValueHint::DirPath)]
    pub browser_profile: Option<PathBuf>,

    /// Dump every provider generation request to <dir> as a separate JSON file.
    /// Each file contains the complete request payload verbatim.
    #[arg(long, value_hint = clap::ValueHint::DirPath)]
    pub dump_requests: Option<PathBuf>,

    /// The subcommand to run. If omitted, launches the TUI.
    #[command(subcommand)]
    pub command: Option<Commands>,
}

impl Cli {
    /// Returns the database path if provided (always `Some` in debug builds).
    pub fn db_path_opt(&self) -> Option<&PathBuf> {
        #[cfg(debug_assertions)]
        {
            Some(&self.db_path)
        }
        #[cfg(not(debug_assertions))]
        {
            self.db_path.as_ref()
        }
    }
}

/// Available subcommands.
#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Launch the TUI (default when no subcommand is given).
    Tui,

    /// Run without a terminal interface.
    #[cfg(debug_assertions)]
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

    /// Install default themes, personas, prompts, and skills to user directories.
    /// Skips any resource that already exists unless --force is given.
    Install {
        /// Overwrite existing resources if they already exist.
        #[arg(long)]
        force: bool,
    },

    /// Fetch reference data from external sources.
    Fetch {
        /// The fetch subcommand to run.
        #[command(subcommand)]
        subcommand: FetchCommands,
    },

    /// Manage user configuration files.
    Config {
        /// The config subcommand to run.
        #[command(subcommand)]
        subcommand: ConfigCommands,
    },

    /// Author, install, and manage wasm plugins.
    Plugin {
        /// The plugin subcommand to run.
        #[command(subcommand)]
        subcommand: PluginCommands,
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

/// Config subcommands.
#[derive(Debug, Subcommand)]
pub enum ConfigCommands {
    /// Write the default jinn.toml to disk.
    ///
    /// Refuses to overwrite an existing file unless --force is given.
    Init {
        /// Overwrite the file if it already exists.
        #[arg(long)]
        force: bool,
    },
    /// Write the default commented providers.toml template to disk.
    ///
    /// Refuses to overwrite an existing file unless --force is given.
    Providers {
        /// Overwrite the file if it already exists.
        #[arg(long)]
        force: bool,
    },
}

/// Plugin subcommands.
#[derive(Debug, Subcommand)]
pub enum PluginCommands {
    /// Scaffold a new plugin cargo project in the current directory.
    New {
        /// The plugin name (crate name; also the `[[plugin]]` entry name).
        name: String,
    },

    /// Download the SDK crates for local plugin authoring.
    Sdk {
        /// The SDK version to fetch (defaults to the wire version).
        version: Option<String>,
    },

    /// Install a built `.wasm` payload as a jinn plugin.
    Install {
        /// Path to the built `.wasm` file.
        wasm: String,

        /// The plugin name (defaults to the file stem).
        name: Option<String>,

        /// Grant a preopened directory path (`<config_dir>`/`<data_dir>`
        /// variables allowed). Repeatable; suffix `:w` for writable.
        #[arg(long = "grant", value_name = "PATH[:w]")]
        grants: Vec<String>,
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
        let cli = Cli::parse_from(["jinn", "--db-path", "/tmp/test.db"]);
        assert!(cli.log_file.is_none());
    }

    // Given a global --log-file argument before a subcommand.
    // Then Cli.log_file captures the override.
    #[test]
    fn log_file_flag_global_overrides() {
        let cli = Cli::parse_from([
            "jinn",
            "--db-path",
            "/tmp/test.db",
            "--log-file",
            "/tmp/x.log",
            "tui",
        ]);
        assert_eq!(
            cli.log_file.as_deref(),
            Some(std::path::Path::new("/tmp/x.log"))
        );
    }

    // Given the old --log-dir argument.
    // Then clap rejects it (the flag has been removed).
    #[test]
    fn log_dir_flag_removed() {
        // In debug mode, missing --db-path would trigger first,
        // so provide a dummy path.
        let result =
            Cli::try_parse_from(["jinn", "--db-path", "/tmp/test.db", "--log-dir", "/tmp"]);
        assert!(result.is_err());
    }

    // Given --log-file scoped to the headless subcommand (old shape).
    // Then clap rejects it: the flag is global now, not a subcommand arg.
    #[test]
    fn headless_scoped_log_file_removed() {
        let result = Cli::try_parse_from([
            "jinn",
            "--db-path",
            "/tmp/test.db",
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
            "jinn",
            "--db-path",
            "/tmp/test.db",
            "--log-file",
            "/tmp/x.log",
            "headless",
            "send-chat",
            "hi",
        ]);
        assert_eq!(
            cli.log_file.as_deref(),
            Some(std::path::Path::new("/tmp/x.log"))
        );
    }

    #[cfg(debug_assertions)]
    #[test]
    fn db_path_required_in_debug() {
        // Given no --db-path flag.
        // When parsing.
        let result = Cli::try_parse_from(["jinn"]);
        // Then clap rejects it.
        assert!(result.is_err());
    }

    #[cfg(not(debug_assertions))]
    #[test]
    fn db_path_optional_in_release() {
        // Given no --db-path flag.
        // When parsing.
        let cli = Cli::parse_from(["jinn"]);
        // Then db_path_opt returns None.
        assert!(cli.db_path_opt().is_none());
    }

    #[test]
    fn db_path_opt_returns_some_when_provided() {
        // Given --db-path /tmp/test.db.
        // When parsing.
        let cli = Cli::parse_from(["jinn", "--db-path", "/tmp/test.db"]);
        // Then db_path_opt returns the path.
        assert_eq!(
            cli.db_path_opt().map(std::path::PathBuf::as_path),
            Some(std::path::Path::new("/tmp/test.db"))
        );
    }

    // Given no --browser-profile argument.
    // Then Cli.browser_profile is None (defaults to AppPaths).
    #[test]
    fn browser_profile_flag_defaults_to_none() {
        let cli = Cli::parse_from(["jinn", "--db-path", "/tmp/test.db"]);
        assert!(cli.browser_profile.is_none());
    }

    // Given a --browser-profile argument.
    // Then Cli.browser_profile captures the override.
    #[test]
    fn browser_profile_flag_parses_to_path() {
        let cli = Cli::parse_from([
            "jinn",
            "--db-path",
            "/tmp/test.db",
            "--browser-profile",
            "/tmp/jinn-profiles",
        ]);
        assert_eq!(
            cli.browser_profile.as_deref(),
            Some(std::path::Path::new("/tmp/jinn-profiles"))
        );
    }
}
