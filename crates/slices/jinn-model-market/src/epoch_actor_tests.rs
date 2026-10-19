//! Observable behavior tests for [`EpochMarketActor`]: cache-vs-seed
//! selection, TTL expiry with an injected clock, refresh dropping the
//! cache, and the event stream's shape.

#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::unwrap_used,
    reason = "test code"
)]

use std::io::Write as _;
use std::sync::Arc;
use std::time::Duration;

use jinn_kernel::common::actor_deps::ActorDeps;
use jinn_kernel::common::bus::HarnessServices as _;
use jinn_model_market_msg::EpochLoaded;
use jinn_model_market_msg::EpochSource;
use jinn_model_market_msg::RefreshEpoch;
use jinn_model_market_msg::RefreshEpochForce;
use jinn_model_market_msg::RefreshFailed;
use jinn_slices::cell::TypedCell;
use jinn_testutil::bus_harness::TestHarness;
use jinn_testutil::bus_harness::await_recorded;
use tempfile::TempDir;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

use crate::epoch::SEED_BENCHMARK_METADATA;
use crate::epoch::SEED_ECI_SCORES;
use crate::epoch::SEED_MODEL_METADATA;
use crate::epoch_actor::EpochMarketActor;
use crate::epoch_actor::EpochMarketActorDeps;

/// Installs the process-wide rustls crypto provider for this test
/// binary — reqwest is built `rustls-no-provider`, so without this every
/// `Client::new()` panics with "No provider set". Idempotent; the result
/// is deliberately ignored.
#[ctor::ctor]
fn install_rustls_provider_for_tests() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// A controlled clock owned by one fixture: the test sets `ms` directly.
struct FakeClock {
    ms: Arc<std::sync::atomic::AtomicU64>,
}

impl FakeClock {
    fn new() -> Self {
        Self {
            ms: Arc::new(std::sync::atomic::AtomicU64::new(1_000_000)),
        }
    }

    fn now(&self) -> u64 {
        self.ms.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn advance(&self, ms: u64) {
        self.ms.fetch_add(ms, std::sync::atomic::Ordering::SeqCst);
    }

    /// The dep-shaped callable reading this clock.
    fn dep(&self) -> std::sync::Arc<dyn Fn() -> u64 + Send + Sync> {
        let ms = Arc::clone(&self.ms);
        std::sync::Arc::new(move || ms.load(std::sync::atomic::Ordering::SeqCst))
    }
}

/// The test fixture: harness, temp cache root, data cell, a spawned
/// actor pointed at a controllable URL, and the fixture's own clock.
struct EpochFixture {
    _dir: TempDir,
    harness: TestHarness,
    data_cell: TypedCell<jinn_model_market_msg::MarketDataState>,
    /// The fixture's own AppPaths; the epoch cache dir resolves from it
    /// exactly as the actor's did at spawn.
    paths: jinn_common::app_paths::AppPaths,
    clock: FakeClock,
}

/// Builds the fixture with the actor's zip URL aimed at `zip_url` and
/// the fixture's own clock wired into the deps. The actor's cache dir
/// resolves through `AppPaths::epoch_cache_dir()` — the same one layout
/// constant production uses.
async fn epoch_fixture_at(zip_url: String) -> EpochFixture {
    let dir = jinn_testutil::tempdir().expect("temp dir");
    let harness = TestHarness::new().await;
    let mut services = harness.services().await;
    services.paths = jinn_common::app_paths::AppPaths::new_in(dir.path());
    let data_cell =
        services
            .slices
            .reader::<jinn_model_market_msg::MarketDataState>(
                &jinn_model_market_msg::market_data_slot(),
            )
            .expect("the cell catalog registers the market data slot");
    let paths = services.paths.clone();
    let clock = FakeClock::new();

    let _actor = EpochMarketActor::spawn(
        harness.system(),
        EpochMarketActorDeps {
            deps: ActorDeps {
                services: services.clone(),
            },
            data_cell: data_cell.clone(),
            cache_dir: services.paths.epoch_cache_dir(),
            client: reqwest::Client::new(),
            zip_url,
            now_ms: clock.dep(),
        },
    );

    EpochFixture {
        _dir: dir,
        harness,
        data_cell,
        paths,
        clock,
    }
}

/// Builds a valid `benchmark_data.zip` in memory carrying exactly the
/// three extraction targets (the seed bytes stand in for Epoch's, so the
/// parse round-trips for free).
fn zip_bytes() -> Vec<u8> {
    let buf = std::io::Cursor::new(Vec::new());
    let mut zip = ZipWriter::new(buf);
    zip.start_file(
        "epoch_capabilities_index/eci_scores.csv",
        SimpleFileOptions::default(),
    )
    .expect("start eci_scores member");
    zip.write_all(SEED_ECI_SCORES).expect("write eci member");
    zip.start_file("benchmark_metadata.csv", SimpleFileOptions::default())
        .expect("start metadata member");
    zip.write_all(SEED_BENCHMARK_METADATA)
        .expect("write metadata member");
    zip.start_file("model_metadata.csv", SimpleFileOptions::default())
        .expect("start roster member");
    zip.write_all(SEED_MODEL_METADATA)
        .expect("write roster member");
    zip.finish().expect("finish zip").into_inner()
}

/// Serves one zip body from a mockito server. Returns the server (keep
/// alive for the mock's lifetime) and the mock (assert with `.assert()`).
async fn serve_zip() -> (mockito::ServerGuard, mockito::Mock) {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/data/benchmark_data.zip")
        .with_status(200)
        .with_body(zip_bytes())
        .create_async()
        .await;
    (server, mock)
}

/// The served zip's URL.
fn zip_url(server: &mockito::ServerGuard) -> String {
    format!("{}/data/benchmark_data.zip", server.url())
}

#[rstest::rstest]
#[tokio::test]
async fn first_run_without_network_uses_seed() {
    // Given a fixture with an empty cache dir and an actor whose zip URL
    // points at a port nothing listens on.
    let fixture = epoch_fixture_at("http://127.0.0.1:9/data/benchmark_data.zip".to_owned()).await;
    let epoch_dir = fixture.paths.epoch_cache_dir();
    assert!(!epoch_dir.exists(), "precondition: no cache dir");
    let loads = fixture.harness.spawn_recorder::<EpochLoaded>().await;
    let failures = fixture.harness.spawn_recorder::<RefreshFailed>().await;

    // When a RefreshEpoch arrives (no server up — the request fails).
    fixture.harness.publish(RefreshEpoch).await;
    let reported = await_recorded::<RefreshFailed>(&failures, 1, Duration::from_secs(5)).await;

    // Then the refresh reports its failure…
    assert_eq!(reported.len(), 1);
    assert_eq!(reported[0].what, "epoch");
    // …and the cell resolves the index from the compiled seed bytes —
    // the ECI column still populates, epoch_source = Seed, nothing is
    // written to disk as a download proxy.
    assert!(
        !epoch_dir.exists(),
        "the seed must never be written to disk"
    );
    // And the seed load itself is announced (the ECI column populates).
    let seed_loads = await_recorded::<EpochLoaded>(&loads, 1, Duration::from_secs(5)).await;
    assert_eq!(seed_loads.len(), 1);
    assert_eq!(seed_loads[0].source, EpochSource::Seed);
    // (Guard scoped to the block: cells are parking_lot locks on the
    // render thread; never held across an await.)
    let (index_nonempty, has_opus, source, loading) = {
        let data = fixture.data_cell.read();
        let index_present = data.eci.as_ref().expect("seed data lands in the cell");
        (
            !index_present.is_empty(),
            index_present.get("Claude Opus 5.5").is_some(),
            data.epoch_source,
            data.epoch_loading,
        )
    };
    assert!(index_nonempty);
    assert!(has_opus);
    assert_eq!(source, EpochSource::Seed);
    assert!(!loading);
}

#[rstest::rstest]
#[tokio::test]
async fn fresh_cache_serves_without_a_request() {
    // Given a complete, fresh cache dir (written by a real extraction
    // into the fixture's own cache root) and no reachable download URL.
    let fixture = epoch_fixture_at("http://127.0.0.1:9/data/benchmark_data.zip".to_owned()).await;
    let epoch_dir = fixture.paths.epoch_cache_dir();
    crate::epoch_cache::download_and_extract(&epoch_dir, &zip_bytes(), fixture.clock.now())
        .expect("seed the cache with a real extraction");
    let loads = fixture.harness.spawn_recorder::<EpochLoaded>().await;

    // When a RefreshEpoch arrives with the clock still inside the TTL.
    fixture.harness.publish(RefreshEpoch).await;
    let landed = await_recorded::<EpochLoaded>(&loads, 1, Duration::from_secs(5)).await;

    // Then the cache serves: source = Download, matched = every scored
    // row, total = every row.
    assert_eq!(landed.len(), 1);
    assert_eq!(landed[0].source, EpochSource::Download);
    assert_eq!(landed[0].matched, 275);
    assert_eq!(landed[0].total, 277);
}

#[rstest::rstest]
#[tokio::test]
async fn expired_epoch_ttl_refetches_from_the_served_zip() {
    // Given a complete cache (written into the fixture's own cache root
    // well before the TTL expires) and a mock serving the refetch.
    let (server, mock) = serve_zip().await;
    let fixture = epoch_fixture_at(zip_url(&server)).await;
    let epoch_dir = fixture.paths.epoch_cache_dir();
    let written_at = fixture.clock.now();
    crate::epoch_cache::download_and_extract(&epoch_dir, &zip_bytes(), written_at)
        .expect("seed the cache");
    let loads = fixture.harness.spawn_recorder::<EpochLoaded>().await;

    // When the clock advances past EPOCH_TTL_MS and a RefreshEpoch
    // arrives.
    fixture.clock.advance(crate::epoch_cache::EPOCH_TTL_MS + 1);
    fixture.harness.publish(RefreshEpoch).await;
    let landed = await_recorded::<EpochLoaded>(&loads, 1, Duration::from_secs(5)).await;

    // Then the fetch hit the mock and the extraction re-landed with a
    // fresh stamp. (The mock was created expecting exactly one hit;
    // `assert_async` fails when the count differs.)
    mock.assert_async().await;
    assert_eq!(landed.len(), 1);
    assert_eq!(landed[0].source, EpochSource::Download);
    let stamp_millis = {
        let data = fixture.data_cell.read();
        data.epoch_fetched_at
            .expect("a successful load stamps the cell")
            .as_millisecond()
    };
    assert_eq!(
        u64::try_from(stamp_millis).unwrap_or(0),
        written_at + crate::epoch_cache::EPOCH_TTL_MS + 1,
        "the refetch re-stamps from the injected clock's advanced value"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn refresh_epoch_force_drops_every_cache() {
    // Given a complete cache dir (the fixture's own) and a mock serving
    // the zip.
    let (server, _mock) = serve_zip().await;
    let fixture = epoch_fixture_at(zip_url(&server)).await;
    let epoch_dir = fixture.paths.epoch_cache_dir();
    crate::epoch_cache::download_and_extract(&epoch_dir, &zip_bytes(), fixture.clock.now())
        .expect("seed the cache");
    assert!(epoch_dir.exists(), "precondition: cache present");
    let loads = fixture.harness.spawn_recorder::<EpochLoaded>().await;

    // When a RefreshEpochForce arrives (forced download against the up mock).
    fixture.harness.publish(RefreshEpochForce).await;
    let landed = await_recorded::<EpochLoaded>(&loads, 1, Duration::from_secs(5)).await;

    // Then the old dir was dropped, the download re-extracted exactly
    // the three CSVs, and the cell shows Download.
    assert!(epoch_dir.exists(), "the refresh re-extracts the cache");
    assert_eq!(landed.len(), 1);
    assert_eq!(landed[0].source, EpochSource::Download);
    let names: Vec<String> = std::fs::read_dir(&epoch_dir)
        .expect("cache dir readable")
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .collect();
    for expected in [
        "eci_scores.csv",
        "benchmark_metadata.csv",
        "model_metadata.csv",
        "fetched_at",
    ] {
        assert!(
            names.iter().any(|n| n == expected),
            "extraction must contain {expected}; dir holds {names:?}",
        );
    }
    assert_eq!(names.len(), 4, "extraction writes exactly the four files");
}

#[rstest::rstest]
#[tokio::test]
async fn refresh_epoch_force_reports_a_failing_download_and_the_seed_stands() {
    // Given a cache dir (the fixture's own) and a refresh whose download
    // fails (no server).
    let fixture = epoch_fixture_at("http://127.0.0.1:9/data/benchmark_data.zip".to_owned()).await;
    let epoch_dir = fixture.paths.epoch_cache_dir();
    crate::epoch_cache::download_and_extract(&epoch_dir, &zip_bytes(), fixture.clock.now())
        .expect("seed the cache");
    let loads = fixture.harness.spawn_recorder::<EpochLoaded>().await;

    // When a RefreshEpochForce arrives.
    fixture.harness.publish(RefreshEpochForce).await;
    let landed = await_recorded::<EpochLoaded>(&loads, 1, Duration::from_secs(5)).await;

    // Then the dropped dir is gone (refresh drops even when the later
    // download fails) and the seed stands in the cell.
    assert!(
        !epoch_dir.exists(),
        "a failed refresh must not leave a half cache behind"
    );
    assert_eq!(landed.len(), 1);
    assert_eq!(landed[0].source, EpochSource::Seed);
    let (has_opus, source) = {
        let data = fixture.data_cell.read();
        (
            data.eci
                .as_ref()
                .expect("seed lands")
                .get("Claude Opus 5.5")
                .is_some(),
            data.epoch_source,
        )
    };
    assert!(has_opus);
    assert_eq!(source, EpochSource::Seed);
}

#[rstest::rstest]
#[test]
fn extraction_writes_exactly_the_three_csvs_plus_stamp() {
    // Given a temp cache root and a real zip.
    let dir = jinn_testutil::tempdir().expect("temp dir");
    let epoch_dir = jinn_common::app_paths::AppPaths::new_in(dir.path()).epoch_cache_dir();

    // When extracting.
    crate::epoch_cache::download_and_extract(&epoch_dir, &zip_bytes(), 123)
        .expect("extraction succeeds");

    // Then exactly the three CSVs (plus the stamp) landed, byte-identical
    // to the sources.
    for (name, expected) in [
        ("eci_scores.csv", SEED_ECI_SCORES),
        ("benchmark_metadata.csv", SEED_BENCHMARK_METADATA),
        ("model_metadata.csv", SEED_MODEL_METADATA),
    ] {
        let written = std::fs::read(epoch_dir.join(name)).expect("file written");
        assert_eq!(written, expected, "{name} must be extracted verbatim");
    }
    let stamp = std::fs::read_to_string(epoch_dir.join("fetched_at")).expect("stamp written");
    assert_eq!(stamp, "123");
    assert_eq!(
        std::fs::read_dir(&epoch_dir).expect("dir").count(),
        4,
        "nothing beyond the three CSVs and the stamp"
    );
}

#[rstest::rstest]
#[test]
fn load_fresh_returns_none_when_any_csv_is_missing() {
    // Given a cache dir with only one of the three CSVs.
    let dir = jinn_testutil::tempdir().expect("temp dir");
    let epoch_dir = jinn_common::app_paths::AppPaths::new_in(dir.path()).epoch_cache_dir();
    std::fs::create_dir_all(&epoch_dir).expect("mkdir");
    std::fs::write(epoch_dir.join("fetched_at"), "1000").expect("stamp");
    std::fs::write(epoch_dir.join("eci_scores.csv"), SEED_ECI_SCORES).expect("partial file");

    // When reading fresh inside the TTL.
    let loaded = crate::epoch_cache::load_fresh(&epoch_dir, 1_000, || 2_000);

    // Then nothing loads — a partial cache never serves.
    assert!(loaded.is_none());
}

#[rstest::rstest]
#[test]
fn load_fresh_returns_none_past_the_ttl() {
    // Given a complete, stamped cache dir.
    let dir = jinn_testutil::tempdir().expect("temp dir");
    let epoch_dir = jinn_common::app_paths::AppPaths::new_in(dir.path()).epoch_cache_dir();
    crate::epoch_cache::download_and_extract(&epoch_dir, &zip_bytes(), 1_000).expect("extraction");

    // When reading with now just past the TTL.
    let loaded = crate::epoch_cache::load_fresh(&epoch_dir, 1_000, || 2_001);

    // Then nothing loads.
    assert!(loaded.is_none());
}

#[rstest::rstest]
#[test]
fn load_fresh_serves_a_complete_cache_inside_the_ttl() {
    // Given a complete, stamped cache dir.
    let dir = jinn_testutil::tempdir().expect("temp dir");
    let epoch_dir = jinn_common::app_paths::AppPaths::new_in(dir.path()).epoch_cache_dir();
    crate::epoch_cache::download_and_extract(&epoch_dir, &zip_bytes(), 1_000).expect("extraction");

    // When reading with now just inside the TTL (one tick before it
    // lapses).
    let loaded = crate::epoch_cache::load_fresh(&epoch_dir, 1_000, || 1_999);

    // Then the cache serves with its stamp.
    let cached = loaded.expect("fresh cache serves");
    assert_eq!(cached.fetched_at_ms, 1_000);
    assert_eq!(cached.files.len(), 3);
}

#[rstest::rstest]
#[test]
fn drop_cache_is_success_when_nothing_is_there() {
    // Given a cache root with no epoch dir.
    let dir = jinn_testutil::tempdir().expect("temp dir");
    let epoch_dir = jinn_common::app_paths::AppPaths::new_in(dir.path()).epoch_cache_dir();

    // When dropping.
    // Then the drop succeeds (nothing to do is not an error).
    crate::epoch_cache::drop_cache(&epoch_dir).expect("drop succeeds");
}

#[rstest::rstest]
#[test]
fn parse_seed_produces_the_full_index_with_metadata() {
    // Given the shipped seed bytes.
    // When parsing all three.
    let data = crate::epoch::seed_data().expect("seed parses");

    // Then the index holds every scored version (the seed's two CI-less
    // rows — e.g. `GPT-5` — are skipped but still counted in total) and
    // the metadata tables ride along.
    assert_eq!(data.index.len(), 275);
    assert_eq!(data.total_rows, 277);
    assert!(data.index.get("GPT-5").is_none());
    assert!(data.index.get("Claude Opus 5.5").is_some());
    assert_eq!(data.benchmark_metadata.len(), 91);
    assert!(
        data.benchmark_metadata
            .iter()
            .any(|m| m.benchmark == "GPQA diamond" && m.in_eci && m.random_baseline == 0.25)
    );
    assert!(!data.roster_names.is_empty());
}
