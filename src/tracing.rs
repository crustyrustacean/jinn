//! Tracing initialization for jinn.
//!
//! Sets up the global tracing subscriber based on the application's run mode.
//! In TUI mode, traces are written exclusively to a file (to avoid corrupting
//! the terminal in raw mode). In headless mode, traces are written to BOTH the
//! terminal and a file. The file path is resolved at CLI parse time from the
//! `--log-file` flag, defaulting to the XDG `state_dir` (see `AppPaths::log_path`).

use std::{env, fs::File, path::PathBuf, sync::Arc};

use clap_verbosity_flag::{Verbosity, WarnLevel};
use error_stack::{Report, ResultExt};
use jinn_domain::common::app_info::APP_NAME;
use tracing_subscriber::{EnvFilter, Layer, layer::SubscriberExt, util::SubscriberInitExt};
use wherror::Error;

/// Error type returned when tracing subscriber initialization fails.
#[derive(Debug, Error)]
#[error(debug)]
pub struct TracingInitError;

/// Decides how the tracing subscriber is configured based on run mode.
///
/// Both variants carry a resolved log file path (caller resolves defaults and
/// overrides before constructing this enum).
#[derive(Debug)]
pub enum TracingMode {
    /// TUI mode: file-only logging to avoid corrupting the terminal.
    Tui {
        /// Resolved path to the log file (e.g. `~/.local/state/jinn/jinn.log`).
        log_path: PathBuf,
    },
    /// Headless mode: writes to BOTH terminal and file.
    Headless {
        /// Resolved path to the log file (e.g. `~/.local/state/jinn/jinn.log`).
        log_path: PathBuf,
    },
}

/// Initializes the global tracing subscriber.
///
/// If the `RUST_LOG` environment variable is set, it takes precedence over
/// the verbosity parameter for filtering log output.
///
/// # Arguments
///
/// * `verbosity` - The verbosity level from CLI flags.
/// * `mode` - The [`TracingMode`] controlling where traces are written. The
///   contained `log_path` is opened in append mode; its parent directory is
///   created if it does not exist.
///
/// # Errors
///
/// Returns a [`TracingInitError`] if the log file cannot be opened or its
/// parent directory cannot be created.
///
/// # Panics
///
/// Panics if called more than once or if another tracer has already been set.
pub fn init(
    verbosity: Verbosity<WarnLevel>,
    mode: TracingMode,
) -> Result<(), Report<TracingInitError>> {
    let filter = match env::var("RUST_LOG") {
        Ok(filter_str) => filter_str,
        Err(_) => format!("{APP_NAME}={verbosity}"),
    };

    let log_path = match &mode {
        TracingMode::Tui { log_path } => log_path,
        TracingMode::Headless { log_path } => log_path,
    };

    let logfile = open_log_file(log_path)?;

    match mode {
        TracingMode::Tui { .. } => {
            let file_layer = tracing_subscriber::fmt::layer()
                .with_file(true)
                .with_line_number(true)
                .with_target(true)
                .with_writer(Arc::new(logfile))
                .with_filter(EnvFilter::new(filter));

            tracing_subscriber::registry().with(file_layer).init();
        }
        TracingMode::Headless { .. } => {
            let file_layer: Box<dyn Layer<_> + Send + Sync + 'static> =
                tracing_subscriber::fmt::layer()
                    .with_file(true)
                    .with_line_number(true)
                    .with_target(true)
                    .with_writer(Arc::new(logfile))
                    .with_filter(EnvFilter::new(filter.clone()))
                    .boxed();

            let terminal_layer =
                tracing_subscriber::fmt::layer().with_filter(EnvFilter::new(filter));

            tracing_subscriber::registry()
                .with(file_layer)
                .with(terminal_layer)
                .init();
        }
    }

    tracing::info!("");
    tracing::info!("--- new session started ---");
    tracing::info!("");

    Ok(())
}

/// Creates the parent directory (if needed) and opens the log file for append.
fn open_log_file(path: &std::path::Path) -> Result<File, Report<TracingInitError>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .change_context(TracingInitError)
            .attach_with(|| {
                format!(
                    "failed to create log directory '{}'",
                    parent.display()
                )
            })?;
    }

    File::options()
        .create(true)
        .append(true)
        .open(path)
        .change_context(TracingInitError)
        .attach_with(|| {
            format!(
                "failed to open file '{}' for tracing",
                path.display()
            )
        })
}
