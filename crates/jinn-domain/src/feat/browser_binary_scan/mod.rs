//! Browser binary scan actor — verifies the configured browser binary at startup.
//!
//! Runs once on [`EnvironmentLoaded`] (program start only, not per-session,
//! unlike [`SkillsScanActor`](crate::feat::skills::skills_scan_actor::SkillsScanActor)).
//! It resolves the configured [`BrowserBinary`] via [`resolve_browser_binary`]
//! and publishes the result on the bus.
//!
//! ## Why events, not direct dashboard writes
//!
//! `frontend.dashboard` is sole-owned by
//! [`DiscordStatusActor`](crate::feat::dashboard::status_actor::DiscordStatusActor).
//! To honour the per-sub-struct ownership rule, this actor does **not** write
//! to the dashboard. It publishes [`BrowserBinaryVerified`] or
//! [`BrowserBinaryMissing`]; the dashboard owner (or a dedicated subscriber)
//! is the correct place to surface the status. This is a deliberate divergence
//! from the original plan, annotated in the spec.

use std::sync::Arc;

use kameo::actor::ActorRef;
use kameo::prelude::{Actor, Context, Message};
use serde::{Deserialize, Serialize};

use crate::common::actor_deps::{ActorDeps, BusPublish};
use crate::common::services::Services;
use crate::common::services::bus_service::BusService;
use crate::feat::web_fetch_actor::BrowserBinary;
use crate::init::env_init_actor::EnvironmentLoaded;

pub mod binary_resolver;

pub use binary_resolver::{
    BinaryFamily, BinaryLocator, BinaryResolutionError, SystemBinaryLocator, resolve_browser_binary,
};

/// Dependencies for [`BrowserBinaryScanActor`].
#[derive(Clone)]
pub struct BrowserBinaryScanActorDeps {
    /// Runtime services and bus access.
    pub deps: ActorDeps,
    /// The configured binary selection (read once from prefs at spawn time).
    pub config: BrowserBinary,
    /// Filesystem seam; [`SystemBinaryLocator`] in production, injectable in
    /// tests. Defaults to [`SystemBinaryLocator`] when constructed via
    /// [`BrowserBinaryScanActorDeps::new`].
    pub locator: Arc<dyn BinaryLocator + Send + Sync>,
}

impl BrowserBinaryScanActorDeps {
    /// Production deps with the system filesystem locator.
    #[must_use]
    pub fn new(deps: ActorDeps, config: BrowserBinary) -> Self {
        Self {
            deps,
            config,
            locator: Arc::new(SystemBinaryLocator),
        }
    }
}

/// Verifies the configured browser binary is reachable at program start.
///
/// Subscribes to [`EnvironmentLoaded`] only. On the event, resolves the binary
/// on a blocking thread and publishes the outcome. It does not hold shared
/// [`State`](crate::common::state::State) — the resolution result is
/// communicated entirely via bus events.
pub struct BrowserBinaryScanActor {
    /// Runtime services.
    #[expect(dead_code, reason = "retained for future use / logging access")]
    services: Services,
    /// Bus service for publishing events.
    bus: BusService,
    /// The configured binary selection.
    config: BrowserBinary,
    /// Filesystem seam; `SystemBinaryLocator` in production, injectable in tests.
    locator: Arc<dyn BinaryLocator + Send + Sync>,
}

impl BusPublish for BrowserBinaryScanActor {
    fn bus(&self) -> &BusService {
        &self.bus
    }
}

impl Actor for BrowserBinaryScanActor {
    type Args = BrowserBinaryScanActorDeps;
    type Error = std::convert::Infallible;

    async fn on_start(args: Self::Args, actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        let bus = args.deps.services.bus.clone();
        bus.subscribe::<EnvironmentLoaded, _>(&actor_ref).await;

        Ok(Self {
            services: args.deps.services,
            bus,
            config: args.config,
            locator: args.locator,
        })
    }
}

impl Message<EnvironmentLoaded> for BrowserBinaryScanActor {
    type Reply = ();

    async fn handle(&mut self, _msg: EnvironmentLoaded, _ctx: &mut Context<Self, Self::Reply>) {
        let config = self.config;
        let locator = self.locator.clone();
        let result =
            tokio::task::spawn_blocking(move || resolve_browser_binary(config, locator.as_ref()))
                .await;

        match result {
            Ok(Ok(path)) => {
                tracing::info!(path = %path.display(), "browser binary verified");
                self.publish(BrowserBinaryVerified { path }).await;
            }
            Ok(Err(err)) => {
                tracing::warn!(%err, config = ?config, "browser binary missing");
                let reason = err.to_string();
                self.publish(BrowserBinaryMissing { reason }).await;
            }
            Err(join_err) => {
                tracing::error!("browser binary scan task panicked: {join_err}");
            }
        }
    }
}

/// Emitted when the configured browser binary was found and verified.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserBinaryVerified {
    /// The resolved executable path.
    pub path: std::path::PathBuf,
}

/// Emitted when the configured browser binary could not be resolved.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserBinaryMissing {
    /// Human-readable reason (e.g. "ChromeNotFound").
    pub reason: String,
}

impl crate::common::bus::BusMessage for BrowserBinaryVerified {}
impl crate::common::bus::BusMessage for BrowserBinaryMissing {}

#[cfg(test)]
mod tests;
