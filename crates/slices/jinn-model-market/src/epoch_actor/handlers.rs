//! The epoch actor's handlers — one named function per trigger, plus
//! the shared load pipeline they run.

use std::sync::Arc;

use jinn_kernel::common::actor_deps::BusPublish as _;
use jinn_model_market_msg::EpochLoaded;
use jinn_model_market_msg::EpochSource;
use jinn_model_market_msg::RefreshFailed;

use super::EpochMarketActor;

impl EpochMarketActor {
    /// `RefreshEpoch`: serve the fresh cache, else download, else seed.
    pub(super) async fn handle_refresh_epoch(&mut self) {
        run_epoch_load(self, false).await;
    }

    /// The forced refresh: drop the epoch cache dir unconditionally,
    /// then run the same load pipeline forced to download.
    ///
    /// Two triggers reach this body, and both mean the same thing — the
    /// epoch cache must go and the data must be refetched: the
    /// `RefreshEpochForce` command, and the `MarketModelsRefreshed`
    /// event the models actor emits when the models half of `<c-r>` is
    /// done.
    pub(super) async fn handle_force_refresh(&mut self) {
        if let Err(report) = crate::epoch_cache::drop_cache(&self.cache_dir) {
            self.publish(RefreshFailed {
                what: "epoch".to_owned(),
                message: report.to_string(),
            })
            .await;
        }
        run_epoch_load(self, true).await;
    }

    /// Writes a successful load into the data cell and announces it.
    pub(super) async fn finish_load(
        &self,
        data: crate::epoch::EpochData,
        source: EpochSource,
        fetched_at_ms: u64,
    ) {
        let stamp =
            jiff::Timestamp::from_millisecond(i64::try_from(fetched_at_ms).unwrap_or(i64::MAX))
                .unwrap_or(jiff::Timestamp::MAX);
        let matched = u32::try_from(data.index.len()).unwrap_or(u32::MAX);
        let total = data.total_rows;
        self.data_cell.update(|state| {
            state.eci = Some(Arc::new(data.index));
            state.epoch_fetched_at = Some(stamp);
            state.epoch_loading = false;
            state.epoch_source = source;
        });
        self.publish(EpochLoaded {
            source,
            matched,
            total,
        })
        .await;
    }

    /// Writes a failure into the data cell (the seed data stands) and
    /// announces it.
    pub(super) async fn fail_load(&self, message: String) {
        self.data_cell.update(|state| {
            state.epoch_loading = false;
            state.epoch_source = EpochSource::Seed;
            state.last_error = Some(message.clone());
        });
        self.publish(RefreshFailed {
            what: "epoch".to_owned(),
            message,
        })
        .await;
    }
}

/// The load pipeline: fresh cache → (forced) download → seed.
///
/// `force` is the forced refresh's mandate: skip the fresh-cache read
/// and go straight to the network, so a refresh actually refreshes.
pub(super) async fn run_epoch_load(actor: &EpochMarketActor, force: bool) {
    actor.data_cell.update(|state| state.epoch_loading = true);
    let now = (actor.now_ms)();

    if !force
        && let Some(cached) =
            crate::epoch_cache::load_fresh(&actor.cache_dir, super::EPOCH_TTL_MS, || {
                (actor.now_ms)()
            })
    {
        load_from_bytes(actor, cached.files, now, EpochSource::Download).await;
        return;
    }

    match crate::epoch_cache::fetch_zip(&actor.client, &actor.zip_url).await {
        Ok(zip_bytes) => {
            if let Err(report) =
                crate::epoch_cache::download_and_extract(&actor.cache_dir, &zip_bytes, now)
            {
                // Downloaded but unusable: fall back to the seed and say
                // so — never leave the cell loading, never write a
                // partial extraction.
                fail_to_seed(
                    actor,
                    format!("epoch refresh failed; showing shipped seed: {report}"),
                )
                .await;
                return;
            }
            match crate::epoch_cache::load_any(&actor.cache_dir) {
                Some(cached) => {
                    load_from_bytes(actor, cached.files, now, EpochSource::Download).await;
                }
                None => {
                    fail_to_seed(
                        actor,
                        "epoch refresh failed; showing shipped seed: extraction vanished"
                            .to_owned(),
                    )
                    .await;
                }
            }
        }
        Err(report) => {
            // No network (or a failing origin): the seed stands. The ECI
            // column still populates with epoch_source = Seed.
            fail_to_seed(
                actor,
                format!("epoch refresh failed; showing shipped seed: {report}"),
            )
            .await;
        }
    }
}

/// Reports a failed refresh and lands the compiled seed in the cell, so
/// the ECI column still populates (`first_run_without_network_uses_seed`).
async fn fail_to_seed(actor: &EpochMarketActor, message: String) {
    actor.fail_load(message).await;
    super::load_seed(actor).await;
}

/// Parses the three CSVs and hands a successful load to the actor.
async fn load_from_bytes(
    actor: &EpochMarketActor,
    files: Vec<Vec<u8>>,
    fetched_at_ms: u64,
    source: EpochSource,
) {
    let mut files = files.into_iter();
    let eci_scores = files.next().unwrap_or_default();
    let benchmark_metadata = files.next().unwrap_or_default();
    let model_metadata = files.next().unwrap_or_default();
    match crate::epoch::parse_all(&eci_scores, &benchmark_metadata, &model_metadata) {
        Ok(data) => actor.finish_load(data, source, fetched_at_ms).await,
        Err(report) => {
            fail_to_seed(
                actor,
                format!(
                    "epoch data unusable; showing shipped seed: {}",
                    parse_failure_detail(&report)
                ),
            )
            .await;
        }
    }
}

/// Renders a parse-error report compactly for the status line.
fn parse_failure_detail(report: &error_stack::Report<crate::epoch::EpochParseError>) -> String {
    format!("{report}")
}
