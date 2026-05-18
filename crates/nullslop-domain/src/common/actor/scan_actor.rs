//! Generic scan actor — runs a blocking directory scan on command and emits results.
//!
//! Provides [`ScanActor`] which encapsulates the common pattern shared by
//! skills scanning, prompt template scanning, and persona scanning:
//!
//! 1. Subscribe to one rescan command
//! 2. Take a `PathBuf` scan path from injected context data
//! 3. On command, run a blocking scan via `spawn_blocking`
//! 4. Emit a `Loaded { items, error }` event
//!
//! Also provides [`NoDirectMsg`], a marker type for actors that don't use
//! direct messages (the `Actor::Message` associated type).

use super::{Actor, ActorContext, ActorEnvelope};
use crate::common::app_paths::AppPaths;

/// Marker type for actors that don't accept direct messages.
///
/// Use as `type Message = NoDirectMsg;` in the `Actor` impl.
pub enum NoDirectMsg {}

/// Configuration trait for scan actors.
///
/// Each scan actor implements this trait to define its specific scanning
/// behavior, command subscription, and result handling.
pub trait ScanConfig: Send + 'static {
    /// The output type returned by the blocking scan function.
    type Output: Send + 'static;

    /// Activate the scan config: subscribe to commands and extract extra data.
    ///
    /// Called once during [`ScanActor::activate`], after `AppPaths` has
    /// already been extracted from context.
    fn activate(ctx: &mut ActorContext) -> Self;

    /// Returns `true` if the given command should trigger a rescan.
    ///
    /// Called by [`ScanActor::handle_command`] to decide whether to run the scan.
    fn is_rescan_command(command: &crate::protocol::Command) -> bool;

    /// Run the blocking scan using the given paths.
    ///
    /// Called inside `spawn_blocking`, so this must not access any async runtime.
    fn scan(paths: &AppPaths) -> Self::Output;

    /// Handle a successful scan result.
    ///
    /// Called after `spawn_blocking` returns `Ok(output)`.
    fn on_success(output: Self::Output, config: &Self, ctx: &ActorContext);

    /// Handle a scan task panic.
    ///
    /// Called after `spawn_blocking` returns `Err(join_error)`.
    /// Default implementation logs the error. Override for custom error events.
    fn on_panic(join_error: tokio::task::JoinError, config: &Self, _ctx: &ActorContext) {
        let _ = (config, join_error);
        // Default: just log. Specific actors override to emit error events.
    }
}

/// Generic scan actor that runs a blocking directory scan on command.
///
/// Subscribes to one command type (defined by `C`), takes `AppPaths` from
/// injected context data, and calls `C::scan()` inside `spawn_blocking`.
pub struct ScanActor<C: ScanConfig> {
    /// Application paths for resolving scan directories.
    paths: AppPaths,
    /// Per-actor configuration (extra state, event constructors, etc.).
    config: C,
}

impl<C: ScanConfig + Sync> Actor for ScanActor<C> {
    type Message = NoDirectMsg;

    #[expect(
        clippy::expect_used,
        reason = "AppPaths must be injected via ctx.set_data before activate"
    )]
    fn activate(ctx: &mut ActorContext) -> Self {
        let config = C::activate(ctx);
        let paths = ctx
            .take_data::<AppPaths>()
            .expect("AppPaths must be injected via ctx.set_data()");
        Self { paths, config }
    }

    async fn handle(&mut self, msg: ActorEnvelope<NoDirectMsg>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Command(command) => {
                self.handle_command(&command, ctx).await;
            }
            ActorEnvelope::Event(_) | ActorEnvelope::System(_) | ActorEnvelope::Direct(_) => {}
        }
    }
}

impl<C: ScanConfig> ScanActor<C> {
    /// Dispatches incoming commands.
    ///
    /// Checks [`ScanConfig::is_rescan_command`] and runs the scan if matched.
    async fn handle_command(&mut self, command: &crate::protocol::Command, ctx: &ActorContext) {
        if C::is_rescan_command(command) {
            self.run_scan(ctx).await;
        }
    }

    /// Runs the blocking scan and delegates result handling.
    async fn run_scan(&self, ctx: &ActorContext) {
        let paths = self.paths.clone();
        let result = tokio::task::spawn_blocking(move || C::scan(&paths)).await;

        match result {
            Ok(output) => C::on_success(output, &self.config, ctx),
            Err(join_error) => C::on_panic(join_error, &self.config, ctx),
        }
    }
}
