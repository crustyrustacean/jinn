//! Tracing initialization for jinn.
//!
//! Sets up the global tracing subscriber based on the application's run mode.
//! In TUI mode, traces are written exclusively to a file (to avoid corrupting
//! the terminal in raw mode). In headless mode, traces are written to BOTH the
//! terminal and a file. The file path is resolved at CLI parse time from the
//! `--log-file` flag, defaulting to the XDG `state_dir` (see `AppPaths::log_path`).

use std::{
    env,
    fs::{File, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::Arc,
};

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

/// Derives the dedicated panic-log path as a sibling of the main `log_path`.
///
/// `jinn.log` -> `jinn-panic.log` in the same directory. If `log_path` has no
/// parent directory, the panic log falls back to `jinn-panic.log` in the CWD.
#[must_use]
fn panic_log_path(log_path: &std::path::Path) -> PathBuf {
    match log_path.parent() {
        Some(dir) => dir.join("jinn-panic.log"),
        None => PathBuf::from("jinn-panic.log"),
    }
}

/// Appends one durable panic record to the panic-log file and flushes.
///
/// Each line is `RFC3339 UTC | panic at file:line:col | message`. Extracted as a
/// free function so the append logic is unit-testable without installing a global
/// hook.
fn write_panic_record(path: &std::path::Path, message: &str, location: Option<(&str, u32, u32)>) {
    let location_str = match location {
        Some((file, line, col)) => format!("{file}:{line}:{col}"),
        None => "<unknown>".to_owned(),
    };
    let ts = jiff::Timestamp::now();
    let line = format!("{ts} | panic at {location_str} | {message}");
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{line}");
        let _ = file.flush();
    }
}

/// Installs the global panic hook.
///
/// Appends each panic's message + source location to the dedicated panic-log at
/// `panic_path`, then chains to the previously-installed hook so default stderr
/// output (or any earlier hook) is preserved. Extracted from `init` so the chaining
/// behavior is unit-testable without standing up the full tracing subscriber.
fn install_panic_hook(panic_path: PathBuf) {
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info: &std::panic::PanicHookInfo<'_>| {
        let location = info.location().map(|l| (l.file(), l.line(), l.column()));
        write_panic_record(&panic_path, &info.to_string(), location);
        previous_hook(info);
    }));
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
    let rust_log = env::var("RUST_LOG").ok();
    let filter = match &rust_log {
        Some(filter_str) => filter_str.clone(),
        None => format!("{APP_NAME}={verbosity}"),
    };

    let log_path = match &mode {
        TracingMode::Tui { log_path } => log_path.clone(),
        TracingMode::Headless { log_path } => log_path.clone(),
    };

    let logfile = open_log_file(&log_path)?;

    // In TUI mode the terminal enters raw mode shortly after init, so traces
    // (including RUST_LOG output) go to the file only — never the screen. If the
    // user set RUST_LOG expecting to watch startup, point them at the file or the
    // --log-file flag. (Headless mode already prints to the terminal.)
    if matches!(mode, TracingMode::Tui { .. }) && rust_log.is_some() {
        eprintln!("RUST_LOG is set, but TUI mode writes traces to a file, not the terminal.");
        eprintln!("  run tail -f '{}' in another term", log_path.display());
        eprintln!("  or rerun with --log-file <path> to choose a different file.");
    }

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

    // Install the global panic hook: appends each panic's message + source
    // location to a dedicated `jinn-panic.log` alongside the main log, then
    // chains to the previous hook so default stderr output is preserved.
    install_panic_hook(panic_log_path(&log_path));

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
            .attach_with(|| format!("failed to create log directory '{}'", parent.display()))?;
    }

    File::options()
        .create(true)
        .append(true)
        .open(path)
        .change_context(TracingInitError)
        .attach_with(|| format!("failed to open file '{}' for tracing", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn panic_log_path_is_sibling_of_log_path() {
        // Given a resolved log path /x/jinn.log.
        let log_path = Path::new("/x/jinn.log");

        // When deriving the panic-log path.
        let panic_path = panic_log_path(log_path);

        // Then it is jinn-panic.log in the same directory.
        assert_eq!(panic_path, PathBuf::from("/x/jinn-panic.log"));
    }

    #[test]
    fn panic_log_path_falls_back_to_cwd_when_no_parent() {
        // Given a log path with no parent directory.
        let log_path = Path::new("jinn.log");

        // When deriving the panic-log path.
        let panic_path = panic_log_path(log_path);

        // Then it falls back to jinn-panic.log in the CWD.
        assert_eq!(panic_path, PathBuf::from("jinn-panic.log"));
    }

    #[test]
    fn write_panic_record_appends_message_and_location() {
        // Given a temp directory as the panic-log location.
        let dir = tempfile::tempdir().expect("temp dir");
        let panic_path = dir.path().join("jinn-panic.log");

        // When writing a panic record.
        write_panic_record(&panic_path, "boom in actor", Some(("src/llm.rs", 42, 7)));

        // Then the file contains a line with the message and the location.
        let content = std::fs::read_to_string(&panic_path).expect("read panic log");
        assert!(
            content.contains("boom in actor"),
            "expected the panic message in the file, got: {content}"
        );
        assert!(
            content.contains("src/llm.rs:42:7"),
            "expected the location in the file, got: {content}"
        );
    }

    #[test]
    fn install_panic_hook_chains_to_previous_hook() {
        // Given a sentinel previous hook that sets a flag.
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        let called = Arc::new(AtomicBool::new(false));
        let called_clone = called.clone();
        std::panic::set_hook(Box::new(move |_| {
            called_clone.store(true, Ordering::SeqCst);
        }));

        // When installing our hook (which chains to the sentinel) and then
        // triggering a real panic in a child thread.
        let dir = tempfile::tempdir().expect("create temp dir");
        let panic_path = dir.path().join("jinn-panic.log");
        install_panic_hook(panic_path.clone());
        let _ = std::thread::spawn(|| panic!("chain test")).join();

        // Then the sentinel hook was invoked (chaining works).
        assert!(
            called.load(Ordering::SeqCst),
            "expected the previous hook to be called via chaining"
        );
        // And the panic log was written.
        let content = std::fs::read_to_string(&panic_path).expect("read panic log");
        assert!(content.contains("chain test"));

        // Restore the default hook to avoid leaking our hook into other tests.
        let _ = std::panic::take_hook();
    }
}
