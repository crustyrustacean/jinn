//! `OpenRouterMarketActor` tests — the caching contract's heart.
//!
//! The mandatory named tests live here: warm-cache-no-request,
//! TTL-expiry-refetch, refresh-all-drops, debounce-drops-stale. Every
//! test drives the real actor over the real bus ([`TestHarness`]) with
//! a mockito server as OpenRouter, so "no outbound request" is measured
//! by the mock expecting zero hits, not by an internal flag.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_in_result,
    clippy::indexing_slicing,
    reason = "test code"
)]

use jinn_kernel::common::bus::HarnessServices;
use jinn_model_market_msg::EndpointsLoaded;
use jinn_model_market_msg::FetchEndpoints;
use jinn_model_market_msg::FetchModels;
use jinn_model_market_msg::MarketDataState;
use jinn_model_market_msg::MarketEndpointRow;
use jinn_model_market_msg::MarketUiState;
use jinn_model_market_msg::ModelsLoaded;
use jinn_model_market_msg::RefreshAll;
use jinn_model_market_msg::market_data_slot;
use jinn_model_market_msg::market_ui_slot;
use jinn_slices::Slices;
use jinn_slices::TypedCell;
use jinn_testutil::bus_harness::TestHarness;
use jinn_testutil::bus_harness::await_recorded;
use std::time::Duration;

use super::OpenRouterMarketActor;
use super::OpenRouterMarketActorDeps;

/// The harness wait: past the 250ms debounce, small enough to keep the
/// suite snappy. Tests that wait on a full refresh — two network fetches
/// through a mockito server behind a debounce — use [`REFRESH_SETTLE`]
/// instead: under full-suite parallel load 700ms can expire before the
/// actor's task is scheduled twice, which looked like a dropped cache
/// sweep rather than a slow machine.
const SETTLE: Duration = Duration::from_millis(700);

/// The wait for tests that ride a RefreshAll through the mockito server.
const REFRESH_SETTLE: Duration = Duration::from_secs(5);

/// One wired test world: harness bus, the two cells, mockito URL.
struct World {
    /// The bus harness — publish here, record there.
    harness: TestHarness,
    /// The data cell the actor writes.
    data: TypedCell<MarketDataState>,
    /// The UI cell the actor reads for the debounce sequence.
    ui: TypedCell<MarketUiState>,
    /// The mockito server's root (`http://127.0.0.1:port`).
    #[expect(dead_code, reason = "kept for follow-up tests to name the server")]
    base_url: String,
    /// The market cache dir (a tempdir under the harness).
    #[expect(dead_code, reason = "kept so the tempdir outlives the test")]
    cache_dir: std::path::PathBuf,
}

impl World {
    /// Builds the world and spawns the actor against `base_url` and
    /// `cache_dir`.
    ///
    /// The cells mint through a real `Slices` registry — the same
    /// registration path production's catalog walks — because
    /// `TypedCell::new` is registry-internal by design.
    async fn start(base_url: &str, cache_dir: std::path::PathBuf) -> Self {
        let harness = TestHarness::new().await;
        let deps = harness.actor_deps().await;
        let slices = Slices::new();
        let data = slices
            .register(market_data_slot(), MarketDataState::default())
            .expect("fresh Slices never has the data slot registered");
        let ui = slices
            .register(market_ui_slot(), MarketUiState::default())
            .expect("fresh Slices never has the UI slot registered");
        OpenRouterMarketActor::spawn(
            harness.system(),
            OpenRouterMarketActorDeps {
                deps,
                data: data.clone(),
                ui: ui.clone(),
                base_url: base_url.to_owned(),
                cache_dir: cache_dir.clone(),
            },
        );
        Self {
            harness,
            data,
            ui,
            base_url: base_url.to_owned(),
            cache_dir,
        }
    }

    /// Records `ModelsLoaded` deliveries.
    async fn models_recorder(&self) -> jinn_testutil::bus_harness::Recorder<ModelsLoaded> {
        self.harness.spawn_recorder::<ModelsLoaded>().await
    }

    /// Records `EndpointsLoaded` deliveries.
    async fn endpoints_recorder(&self) -> jinn_testutil::bus_harness::Recorder<EndpointsLoaded> {
        self.harness.spawn_recorder::<EndpointsLoaded>().await
    }
}

/// A minimal one-model catalog body for the mock.
fn catalog_body() -> String {
    serde_json::json!({
        "data": [
            {
                "id": "test/model",
                "pricing": {
                    "prompt": "0.000003",
                    "completion": "0.000015",
                    "input_cache_read": "0.0000003"
                },
                "architecture": {"output_modalities": ["text"]},
                "context_length": 100_000
            }
        ]
    })
    .to_string()
}

/// A minimal one-endpoint detail body for the mock.
fn endpoints_body() -> String {
    serde_json::json!({
        "data": {
            "endpoints": [
                {
                    "provider_name": "TestUp",
                    "tag": "testup",
                    "pricing": {"prompt": "0.000003", "completion": "0.000015"},
                    "uptime_last_1d": 99.0
                }
            ]
        }
    })
    .to_string()
}

/// Registers a models GET mock that expects exactly one hit.
async fn expect_models_once(server: &mut mockito::Server) -> mockito::Mock {
    server
        .mock("GET", "/models")
        .with_status(200)
        .with_body(catalog_body())
        .create_async()
        .await
}

/// Registers an endpoints GET mock that expects exactly one hit.
async fn expect_endpoints_once(server: &mut mockito::Server) -> mockito::Mock {
    server
        .mock("GET", "/models/test/model/endpoints")
        .with_status(200)
        .with_body(endpoints_body())
        .create_async()
        .await
}

#[rstest::rstest]
#[tokio::test]
async fn warm_models_cache_makes_no_outbound_request() {
    // Given a disk cache written by a previous run and a mock server
    // with NO registered route: mockito answers 501 to any outbound
    // request, so a hidden fetch fails and never emits ModelsLoaded.
    let server = mockito::Server::new_async().await;
    let dir = tempfile::TempDir::new().expect("temp dir");
    let at = jiff::Timestamp::now() - jiff::SignedDuration::from_hours(1);
    crate::market_cache::store(
        &dir.path().join("models.json"),
        &crate::market_cache::stamped(fixture_rows(), at),
    )
    .expect("seed the warm cache");
    let world = World::start(&server.url(), dir.path().to_path_buf()).await;
    let recorder = world.models_recorder().await;

    // When publishing FetchModels against the request-less server.
    world.harness.publish(FetchModels).await;
    let loaded = await_recorded(&recorder, 1, SETTLE).await;

    // Then the load lands from the warm cache: the event arriving at
    // all is the outbound-request proof — a request would have 501'd.
    assert_eq!(loaded.len(), 1, "exactly one ModelsLoaded");
    assert!(loaded[0].from_cache, "warm cache must set from_cache");
    let rows = world.data.read().rows.clone();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, "openrouter/test/model");
}

#[rstest::rstest]
#[tokio::test]
async fn expired_models_ttl_refetches() {
    // Given a disk cache stamped 8 days ago — past the 7-day TTL — and
    // a mock expecting exactly one GET.
    let mut server = mockito::Server::new_async().await;
    let dir = tempfile::TempDir::new().expect("temp dir");
    let at = jiff::Timestamp::now() - jiff::SignedDuration::from_hours(8 * 24);
    crate::market_cache::store(
        &dir.path().join("models.json"),
        &crate::market_cache::stamped(fixture_rows(), at),
    )
    .expect("seed the stale cache");
    let world = World::start(&server.url(), dir.path().to_path_buf()).await;
    let recorder = world.models_recorder().await;
    let mock = expect_models_once(&mut server).await;

    // When publishing FetchModels.
    world.harness.publish(FetchModels).await;
    let loaded = await_recorded(&recorder, 1, SETTLE).await;

    // Then the expiry refetches: the single allowed GET was consumed
    // and the event says not-from-cache.
    assert_eq!(loaded.len(), 1, "exactly one ModelsLoaded");
    assert!(!loaded[0].from_cache, "a refetch is not from_cache");
    assert_eq!(world.data.read().rows.len(), 1);
    mock.assert_async().await;
}

#[rstest::rstest]
#[tokio::test]
async fn epoch_loaded_rejoins_rows_without_a_refetch() {
    // Given an actor that fetched models while the index was absent —
    // rows built with dead ECI fields — and a models mock that would
    // FAIL if hit again (the re-join must not refetch). The catalog
    // carries a real seed joinable (Claude Sonnet 4.5, dot-vs-space).
    let mut server = mockito::Server::new_async().await;
    let dir = tempfile::TempDir::new().expect("temp dir");
    let joinable_catalog = serde_json::json!({
        "data": [
            {
                "id": "anthropic/claude-sonnet-4.5",
                "pricing": {"prompt": "0.000003", "completion": "0.000015"},
                "architecture": {"output_modalities": ["text"]},
                "context_length": 100_000
            }
        ]
    })
    .to_string();
    let models_mock = server
        .mock("GET", "/models")
        .with_status(200)
        .with_body(joinable_catalog)
        .create_async()
        .await;
    let world = World::start(&server.url(), dir.path().to_path_buf()).await;
    let recorder = world.models_recorder().await;
    world.harness.publish(FetchModels).await;
    let loaded = await_recorded(&recorder, 1, SETTLE).await;
    assert!(!loaded[0].from_cache);
    let before = world.data.read().rows.clone();
    assert!(
        before.iter().all(|r| r.eci.is_none()),
        "precondition: rows joined against an empty index"
    );

    // When the epoch actor lands the index in the cell and publishes
    // EpochLoaded — the raw display-name key form the real epoch uses.
    let index = crate::epoch::parse_eci_scores(crate::epoch::SEED_ECI_SCORES)
        .expect("seed parses")
        .0;
    world
        .data
        .update(|d| d.eci = Some(std::sync::Arc::new(index)));
    world
        .harness
        .publish(jinn_model_market_msg::EpochLoaded {
            source: jinn_model_market_msg::EpochSource::Seed,
            matched: 1,
            total: 1,
        })
        .await;
    tokio::time::sleep(SETTLE).await;

    // Then the rows in the cell carry ECI scores — no second fetch.
    models_mock.assert_async().await;
    let after = world.data.read();
    assert!(
        after.rows.iter().any(|r| r.eci.is_some()),
        "EpochLoaded must re-join the retained raw listings"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn refresh_all_drops_every_cache() {
    // Given warm models AND endpoints caches at both levels, with mocks
    // expecting exactly one GET each for the refetch.
    let mut server = mockito::Server::new_async().await;
    let dir = tempfile::TempDir::new().expect("temp dir");
    let at = jiff::Timestamp::now() - jiff::SignedDuration::from_hours(1);
    let detail = vec![MarketEndpointRow {
        tag: "stale".to_owned(),
        provider_name: "Stale".to_owned(),
        prompt_price: 1.0,
        completion_price: 1.0,
        cache_read_price: None,
        latency_p50_ms: None,
        throughput_p50: None,
        uptime_1d: None,
        quantization: None,
        overrides: Vec::new(),
    }];
    // A SECOND detail file for a model that is NOT displayed — refresh
    // must sweep the whole cache dir, not just the visible pane's file.
    crate::market_cache::store(
        &dir.path().join("models.json"),
        &crate::market_cache::stamped(fixture_rows(), at),
    )
    .expect("seed models cache");
    crate::market_cache::store(
        &dir.path().join("endpoints-test_model.json"),
        &crate::market_cache::stamped(detail.clone(), at),
    )
    .expect("seed endpoints cache");
    crate::market_cache::store(
        &dir.path().join("endpoints-other_model.json"),
        &crate::market_cache::stamped(detail, at),
    )
    .expect("seed the non-displayed endpoint cache");
    let world = World::start(&server.url(), dir.path().to_path_buf()).await;
    // Put a detail id on the data cell so refresh knows what to drop.
    let displayed = jinn_model_market_msg::MarketDetail {
        model_id: "openrouter/test/model".to_owned(),
        endpoints: Vec::new(),
    };
    world.data.update(|d| d.detail = Some(displayed));
    let recorder = world.models_recorder().await;
    let models_mock = expect_models_once(&mut server).await;
    let endpoints_mock = expect_endpoints_once(&mut server).await;

    // When publishing RefreshAll.
    world.harness.publish(RefreshAll).await;
    let loaded = await_recorded(&recorder, 1, REFRESH_SETTLE).await;

    // Then both GETs were consumed — memory and disk caches are gone.
    assert_eq!(loaded.len(), 1, "models refetched");
    assert!(!loaded[0].from_cache);
    models_mock.assert_async().await;
    endpoints_mock.assert_async().await;
    // And the sweep dropped every endpoint disk file — the only one
    // present is the displayed model's freshly refetched file.
    let leftovers: Vec<String> = std::fs::read_dir(dir.path())
        .expect("cache dir readable")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("endpoints-"))
        .collect();
    assert_eq!(
        leftovers,
        vec!["endpoints-openrouter_test_model.json".to_owned()],
        "refresh must sweep ALL endpoint files, then the displayed refetch rewrites only its own"
    );
    // And the detail was refetched too, replacing the stale cell entry.
    let detail = world.data.read().detail.clone().expect("detail refetched");
    assert_eq!(detail.model_id, "openrouter/test/model");
    assert_eq!(detail.endpoints[0].tag, "testup");
}

#[rstest::rstest]
#[tokio::test]
async fn endpoint_debounce_drops_stale_fetches() {
    // Given an actor with an empty cache and a mock for the NEWEST
    // model only — a hit for the stale model would fail the test.
    let mut server = mockito::Server::new_async().await;
    let dir = tempfile::TempDir::new().expect("temp dir");
    let world = World::start(&server.url(), dir.path().to_path_buf()).await;
    let recorder = world.endpoints_recorder().await;
    let newest = expect_endpoints_once(&mut server).await;

    // When two fetches race: seq 1 for the stale model, seq 2 for the
    // newest (the UI cell's sequence reads 2 when the debounce wakes).
    world.ui.update(|s| s.fetch_seq = 2);
    world
        .harness
        .publish(FetchEndpoints {
            model_id: "openrouter/stale/model".to_owned(),
            seq: 1,
        })
        .await;
    world
        .harness
        .publish(FetchEndpoints {
            model_id: "openrouter/test/model".to_owned(),
            seq: 2,
        })
        .await;
    let loaded = await_recorded(&recorder, 1, SETTLE).await;

    // Then only the newest model fetched: one EndpointsLoaded for it,
    // none for the stale one, and the single allowed GET was consumed.
    assert_eq!(loaded.len(), 1, "only the newest fetch lands");
    assert_eq!(loaded[0].model_id, "openrouter/test/model");
    let detail = world.data.read().detail.clone().expect("detail written");
    assert_eq!(detail.model_id, "openrouter/test/model");
    newest.assert_async().await;
}

#[rstest::rstest]
#[tokio::test]
async fn equal_seq_fetch_still_serves_after_the_debounce() {
    // Given an actor whose UI sequence has not advanced since the
    // request was minted.
    let mut server = mockito::Server::new_async().await;
    let dir = tempfile::TempDir::new().expect("temp dir");
    let world = World::start(&server.url(), dir.path().to_path_buf()).await;
    let recorder = world.endpoints_recorder().await;
    let mock = expect_endpoints_once(&mut server).await;

    // When a single fetch runs with the sequence unchanged.
    world
        .harness
        .publish(FetchEndpoints {
            model_id: "openrouter/test/model".to_owned(),
            seq: 0,
        })
        .await;
    let loaded = await_recorded(&recorder, 1, SETTLE).await;

    // Then the fetch is served — only strictly older sequences drop.
    assert_eq!(loaded.len(), 1);
    mock.assert_async().await;
}

#[rstest::rstest]
#[tokio::test]
async fn fetch_models_without_a_key_still_serves_the_table() {
    // Given a world whose API-key store carries no OPENROUTER_API_KEY
    // (the harness's default) and a mock that requires NO auth header.
    let mut server = mockito::Server::new_async().await;
    let dir = tempfile::TempDir::new().expect("temp dir");
    let world = World::start(&server.url(), dir.path().to_path_buf()).await;
    let recorder = world.models_recorder().await;
    let mock = server
        .mock("GET", "/models")
        .match_header("authorization", mockito::Matcher::Missing)
        .with_status(200)
        .with_body(catalog_body())
        .create_async()
        .await;

    // When publishing FetchModels.
    world.harness.publish(FetchModels).await;
    let loaded = await_recorded(&recorder, 1, SETTLE).await;

    // Then the unauthenticated fetch serves the table — a missing key
    // must not gate /models.
    assert_eq!(loaded.len(), 1);
    assert!(!loaded[0].from_cache);
    mock.assert_async().await;
}

#[rstest::rstest]
#[tokio::test]
async fn fetch_models_failure_emits_refresh_failed_and_keeps_stale_rows() {
    // Given an actor whose first fetch succeeded, then the mock breaks.
    let mut server = mockito::Server::new_async().await;
    let dir = tempfile::TempDir::new().expect("temp dir");
    let world = World::start(&server.url(), dir.path().to_path_buf()).await;
    world.data.update(|d| {
        d.rows = fixture_rows();
    });

    // When the models endpoint 500s.
    server
        .mock("GET", "/models")
        .with_status(500)
        .with_body("boom")
        .create_async()
        .await;
    world.harness.publish(FetchModels).await;
    tokio::time::sleep(SETTLE).await;

    // Then the stale rows stand and the error line is set.
    let d = world.data.read();
    assert_eq!(d.rows.len(), 1, "stale rows must not be dropped");
    assert!(d.last_error.is_some(), "the degraded-mode line is set");
}

/// One-row fixture matching [`catalog_body`]'s listing.
fn fixture_rows() -> Vec<jinn_model_market_msg::MarketRow> {
    vec![jinn_model_market_msg::MarketRow {
        id: "openrouter/test/model".to_owned(),
        author: "test".to_owned(),
        slug: "model".to_owned(),
        variant: None,
        prompt_price: 3e-6,
        completion_price: 1.5e-5,
        cache_read_price: Some(3e-7),
        has_overrides: false,
        context_length: Some(100_000),
        eci: None,
        eci_ci: None,
        is_free: false,
    }]
}
