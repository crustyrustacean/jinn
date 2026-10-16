//! Model-market actor-subscription composition tests.
//!
//! The popup is live-wired (cells, routes, view) but only useful once its
//! two data-plane actors are on the bus — the whole slice shipped exactly
//! that way once: subscribed to nothing, so the opener's `FetchModels`
//! publish dropped and the table showed seed data forever. These tests
//! spawn the actors the way `src/bootstrap/slices.rs` does
//! (`jinn_model_market::activate_actors` over a catalog-seeded `Services`)
//! and prove a published command reaches a live subscriber.
//!
//! Hermetic by construction: the OpenRouter actor's first `FetchModels`
//! runs warm-cache-or-network against a cache dir the fake `Services`
//! points at a tempdir, and the epoch actor's URL is the production
//! constant against a port nothing listens on. What "subscribed" means
//! here is an observable cell/event outcome, not an HTTP success.

#![allow(clippy::expect_used, clippy::panic, reason = "test code")]

use std::time::Duration;

use jinn_model_market_msg::EpochLoaded;
use jinn_model_market_msg::EpochSource;
use jinn_model_market_msg::RefreshEpoch;
use jinn_model_market_msg::RefreshFailed;
use jinn_model_market_msg::market_data_slot;
use jinn_slices::TypedCell;
use jinn_testutil::bus_harness::await_recorded;

/// The settle window: past the 250ms endpoint debounce, small enough to
/// keep the suite snappy.
const SETTLE: Duration = Duration::from_millis(700);

/// Installs the process-wide rustls crypto provider for this test
/// binary — reqwest is built `rustls-no-provider`, so without this every
/// `Client::new()` panics with "No provider set". Idempotent; the result
/// is deliberately ignored.
#[ctor::ctor]
fn install_rustls_provider_for_tests() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Composition the way the boot list wires it: catalog-seeded
/// `Services`, `activate` for the cells/routes/view, `activate_actors` for
/// the subscriptions. Endpoints are dead URLs — the claim under test is
/// "subscribed", not "online", so the tests must not depend on the
/// network being down (or, on a live one, up).
async fn composed() -> jinn_kernel::Services {
    let services = jinn_kernel::Services::new_fake().await;
    let mut viewport = jinn_slices::view::Viewport::new();
    let mut host = jinn_slices::SliceHost::new(
        &services.slices,
        &mut viewport,
        &services.overlay_views,
        &services.key_routes,
        &services.trouper_system,
    );
    jinn_model_market::activate(&mut host);
    jinn_model_market::activate_actors_with(
        &services,
        jinn_model_market::ActorEndpoints {
            zip_url: "http://127.0.0.1:9/data/benchmark_data.zip".to_owned(),
            base_url: "http://127.0.0.1:9".to_owned(),
        },
    );
    services
}

/// The data cell handle from the composed services.
fn data_of(services: &jinn_kernel::Services) -> TypedCell<jinn_model_market_msg::MarketDataState> {
    services
        .slices
        .reader(&market_data_slot())
        .expect("the cell catalog registers the market data slot")
}

#[rstest::rstest]
#[tokio::test]
#[timeout(Duration::from_secs(20))]
async fn published_fetch_models_reaches_the_live_actor() {
    // Given the slice composed as the boot list composes it: routes and
    // view attached, both actors spawned over the catalog-seeded cells.
    let services = composed().await;

    // When the opener's FetchModels publish rides the real bus.
    services
        .bus
        .publish(jinn_model_market_msg::FetchModels)
        .await;
    tokio::time::sleep(SETTLE).await;

    // Then the actor answered: the empty cache dir forces the network
    // path, which fails offline — the degraded-mode line lands in the
    // cell. Either way the message was consumed, not dropped
    // unsubscribed (zero outcomes is the dead-bus signature).
    let data = data_of(&services);
    assert!(
        data.read().models_fetched_at.is_some() || data.read().last_error.is_some(),
        "a published FetchModels must reach the live actor and land an outcome \
         in the data cell, got: models_fetched_at={:?} last_error={:?}",
        data.read().models_fetched_at,
        data.read().last_error
    );
}

#[rstest::rstest]
#[tokio::test]
#[timeout(Duration::from_secs(20))]
async fn published_refresh_epoch_reaches_the_live_actor_offline_seed_stands() {
    // Given the composed slice whose epoch actor's zip URL points at the
    // production constant but whose request has no listener.
    let services = composed().await;
    let recorder = jinn_testutil::bus_harness::TestHarness::from_parts(
        services.bus.clone(),
        services.trouper_system.clone(),
    )
    .spawn_recorder::<EpochLoaded>()
    .await;

    // When a RefreshEpoch arrives (the boot path's lazy weekly check
    // publishes this shape; here the download cannot succeed).
    services.bus.publish(RefreshEpoch).await;
    let loaded = await_recorded::<EpochLoaded>(&recorder, 1, Duration::from_secs(5)).await;

    // Then the epoch actor answered with the seed fallback — the ECI
    // column still populates with no network, exactly the first-run
    // contract.
    assert_eq!(loaded.len(), 1, "one EpochLoaded per RefreshEpoch");
    assert_eq!(loaded[0].source, EpochSource::Seed);
    let data = data_of(&services);
    assert!(data.read().eci.is_some(), "the seed ECI index landed");
}

#[rstest::rstest]
#[tokio::test]
#[timeout(Duration::from_secs(20))]
async fn warm_models_cache_serves_the_popup_without_network() {
    // Given the composed slice with a warm models disk cache in the
    // fake Services' tempdir.
    let services = composed().await;
    let cache_dir = services.paths.openrouter_cache_dir();
    std::fs::create_dir_all(&cache_dir).expect("cache dir creates");
    let rows: Vec<jinn_model_market_msg::MarketRow> = vec![jinn_model_market_msg::MarketRow {
        id: "openrouter/a/small".to_owned(),
        author: "a".to_owned(),
        slug: "small".to_owned(),
        variant: None,
        prompt_price: 0.000_001,
        completion_price: 0.000_002,
        cache_read_price: None,
        has_overrides: false,
        context_length: Some(128_000),
        eci: None,
        eci_ci: None,
        is_free: false,
    }];
    let now = jiff::Timestamp::now();
    jinn_model_market::market_cache::store(
        &cache_dir.join("models.json"),
        &jinn_model_market::market_cache::stamped(rows, now),
    )
    .expect("warm cache writes");

    // When the opener's FetchModels publish rides the bus.
    services
        .bus
        .publish(jinn_model_market_msg::FetchModels)
        .await;
    tokio::time::sleep(SETTLE).await;

    // Then the actor served the cache into the cell — no fetch failure
    // recorded, the row on the table.
    let data = data_of(&services);
    let read = data.read();
    assert_eq!(
        read.rows.first().map(|r| r.id.as_str()),
        Some("openrouter/a/small"),
        "the warm cache's row lands in the cell, got: {:?}",
        read.rows
    );
    assert!(
        read.last_error.is_none(),
        "a warm cache never reports a fetch failure, got: {:?}",
        read.last_error
    );
}

#[rstest::rstest]
#[tokio::test]
#[timeout(Duration::from_secs(20))]
async fn refresh_failure_reports_through_the_bus() {
    // Given the composed slice with no cache and no network (the OR
    // actor's base URL is production's, unreachable in the test), and
    // recorders subscribed before anything publishes.
    let services = composed().await;
    let harness = jinn_testutil::bus_harness::TestHarness::from_parts(
        services.bus.clone(),
        services.trouper_system.clone(),
    );
    let failures = harness.spawn_recorder::<RefreshFailed>().await;
    let loaded = harness.spawn_recorder::<EpochLoaded>().await;

    // When a RefreshAll arrives (the `<c-r>` publish): both actors
    // consume it, the epoch one falls back to seed, the OR one fails
    // its fetch and reports.
    services
        .bus
        .publish(jinn_model_market_msg::RefreshAll)
        .await;
    let reported = await_recorded::<RefreshFailed>(&failures, 1, Duration::from_secs(10)).await;

    // Then the OR actor's failure is announced on the bus (the
    // degraded-mode path), and the epoch actor still answers with seed.
    assert!(
        reported.iter().any(|f| f.what == "models"),
        "the models fetch failure is announced, got: {reported:?}"
    );
    let epoch = await_recorded::<EpochLoaded>(&loaded, 1, Duration::from_secs(5)).await;
    assert_eq!(epoch[0].source, EpochSource::Seed);
}
