//! The epoch actor — loads, caches, and refreshes the epoch benchmark
//! data off the key thread.
//!
//! Handles [`RefreshEpoch`] and [`RefreshAll`]; writes only the
//! [`MarketDataState`] cell (`eci`, `epoch_fetched_at`, `epoch_loading`,
//! `epoch_source`, `last_error`); emits [`EpochLoaded`] and
//! [`RefreshFailed`].
//!
//! Selection order per refresh: a fresh disk cache (within
//! [`EPOCH_TTL_MS`], injectable clock) serves without a request; a
//! missing or stale cache triggers a download-and-extract; a failed
//! download falls back to the compiled seed — the ECI column still
//! populates and the status line reports the source. The seed bytes are
//! never written to disk as a download stand-in.
//!
//! RefreshAll drops the epoch cache dir first, then forces the download
//! path — refresh means refresh — and reports a failed drop as
//! `RefreshFailed` without skipping the download attempt.

use jinn_kernel::common::actor_deps::ActorDeps;
use jinn_kernel::common::actor_deps::BusPublish;
use jinn_model_market_msg::EpochLoaded;
use jinn_model_market_msg::EpochSource;
use jinn_model_market_msg::MarketDataState;
use jinn_model_market_msg::RefreshAll;
use jinn_model_market_msg::RefreshEpoch;
use jinn_model_market_msg::RefreshFailed;
use jinn_slices::cell::TypedCell;
use trouper::actor::ActorPath;
use trouper::actor::MsgHandler;
use trouper::actor::ServiceActor;
use trouper::context::MsgCtx;
use trouper::registry::RegistryError;
use trouper::system::ActorSystem;

use crate::epoch;

/// The epoch actor's static trouper path.
pub const EPOCH_ACTOR_PATH: &str = "model-market-epoch";

/// The cache TTL, re-exported beside the actor that enforces it.
pub use crate::epoch_cache::EPOCH_TTL_MS;

/// Dependencies for [`EpochMarketActor`].
#[derive(Clone)]
pub struct EpochMarketActorDeps {
    /// Universal actor deps (services including the bus).
    pub deps: ActorDeps,
    /// The market data cell — this actor is its epoch-fields writer.
    pub data_cell: TypedCell<MarketDataState>,
    /// The resolved epoch cache directory (`AppPaths::epoch_cache_dir()`),
    /// passed directly so the actor cannot re-derive a divergent path.
    pub epoch_cache_dir: std::path::PathBuf,
    /// The HTTP client for the zip download (tests point it at a mock).
    pub client: reqwest::Client,
    /// The epoch zip URL (tests point it at a mock server).
    pub zip_url: String,
    /// Injectable clock, ms since the epoch. Production passes
    /// [`epoch_market_clock`]; tests pass a controlled closure.
    pub now_ms: std::sync::Arc<dyn Fn() -> u64 + Send + Sync>,
}

/// The epoch actor: cache-or-download-or-seed for the benchmark data.
pub struct EpochMarketActor {
    /// Universal actor deps (the bus lives here).
    deps: ActorDeps,
    /// The market data cell — the only cell this actor writes.
    data_cell: TypedCell<MarketDataState>,
    /// Resolved epoch cache dir (the deps' `epoch_cache_dir`).
    cache_dir: std::path::PathBuf,
    /// The HTTP client for the zip download.
    client: reqwest::Client,
    /// The epoch zip URL.
    zip_url: String,
    /// Injectable clock.
    now_ms: std::sync::Arc<dyn Fn() -> u64 + Send + Sync>,
}

impl ServiceActor for EpochMarketActor {
    #[expect(
        clippy::unused_async_trait_impl,
        reason = "trait contract: start is never called (spawn uses start_with)"
    )]
    async fn start(
        _args: &trouper::json::Json,
    ) -> Result<Self, error_stack::Report<RegistryError>> {
        // Never called: the spawn helper injects the deps via `start_with`.
        Err(
            error_stack::IntoReport::into_report(RegistryError::InvalidSpec)
                .attach("EpochMarketActor is spawned via start_with"),
        )
    }
}

impl BusPublish for EpochMarketActor {
    fn bus(&self) -> &jinn_kernel::common::services::bus_service::BusService {
        &self.deps.services.bus
    }
}

impl EpochMarketActor {
    /// Spawns the actor at its static path and returns the path.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "port convention: spawn takes owned deps and clones into start_with"
    )]
    pub fn spawn(system: &ActorSystem, actor_deps: EpochMarketActorDeps) -> ActorPath {
        let path = ActorPath::new(EPOCH_ACTOR_PATH);
        trouper::builder::spawn_service_builder::<Self>(system)
            .at(path.clone())
            .start_with({
                let actor_deps = actor_deps.clone();
                move || {
                    let actor_deps = actor_deps.clone();
                    Box::pin(async move {
                        Ok(Self {
                            deps: actor_deps.deps,
                            data_cell: actor_deps.data_cell,
                            cache_dir: actor_deps.epoch_cache_dir,
                            client: actor_deps.client,
                            zip_url: actor_deps.zip_url,
                            now_ms: actor_deps.now_ms,
                        })
                    })
                }
            })
            // Both command subscriptions and BOTH event emissions are
            // load-bearing: a message the actor handles but does not
            // declare is never delivered; an event it sends but does not
            // declare is dead-lettered at flush and never routes.
            .handles::<RefreshEpoch>()
            .handles::<RefreshAll>()
            .emits::<EpochLoaded>()
            .emits::<RefreshFailed>()
            .start();
        path
    }
}

impl MsgHandler<RefreshEpoch> for EpochMarketActor {
    async fn handle(&mut self, _msg: &RefreshEpoch, _ctx: &mut MsgCtx<'_>) {
        self.handle_refresh_epoch().await;
    }
}

impl MsgHandler<RefreshAll> for EpochMarketActor {
    async fn handle(&mut self, _msg: &RefreshAll, _ctx: &mut MsgCtx<'_>) {
        self.handle_refresh_all().await;
    }
}

mod handlers;

#[cfg(test)]
#[path = "epoch_actor_tests.rs"]
mod epoch_actor_tests;

/// Parses the seed bytes and lands them in the cell — the first-run
/// fallback. The seed always parses (it is a compile-time fixture); the
/// `Err` arm reports a `RefreshFailed` naming the parse bug.
pub async fn load_seed(actor: &EpochMarketActor) {
    match epoch::seed_data() {
        Ok(data) => {
            let now = (actor.now_ms)();
            actor.finish_load(data, EpochSource::Seed, now).await;
        }
        Err(report) => {
            actor
                .fail_load(format!("shipped epoch seed failed to parse — {report}"))
                .await;
        }
    }
}

/// The production clock: real wall time, ms since the epoch.
#[must_use]
pub fn epoch_market_clock() -> std::sync::Arc<dyn Fn() -> u64 + Send + Sync> {
    std::sync::Arc::new(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64)
    })
}
