//! Unit tests for `HeadlessChromeFetcher`.
//!
//! The fetcher is tested via a fake [`HeadlessBrowserFactory`] that scripts a
//! sequence of browser behaviors (success, connection death, render error).
//! This drives the launch / eviction / retry state machine without spawning
//! a real Chromium.
//!
// Fakes intentionally panic on poisoned locks or behavior underflow —
// silent failure would mask broken test setup. These lints do not apply.
#![allow(
    clippy::unwrap_in_result,
    clippy::panic_in_result_fn,
    clippy::expect_used,
    clippy::panic,
    reason = "test fakes intentionally panic on poisoned locks or behavior underflow"
)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use parking_lot::Mutex as ParkingMutex;

use super::{
    HeadlessBrowser, HeadlessBrowserFactory, RenderedPage, build_launch_options,
    classify_browser_error, classify_render_error, extract_content, fetch_once,
};
use crate::stealth::StealthSettings;
use crate::{Extractor, FetchError, FetchOptions, OutputFormat, WebFetcher};

// ---------------------------------------------------------------------------
// Test fakes
// ---------------------------------------------------------------------------

/// A render outcome the fake browser should produce for one `render` call.
#[derive(Clone)]
enum RenderOutcome {
    Ok(RenderedPage),
    /// Produces the exact `ConnectionClosed` message (maps to `BrowserCrash`).
    Crash,
    /// An unrelated per-tab failure (stays `Render`).
    RenderError(String),
    /// The closure panics when invoked (simulates a chrome-internal unwind).
    Panic(String),
}

/// A fake browser that serves a queued outcome per `render` call, LIFO.
struct FakeBrowser {
    behaviors: Mutex<Vec<RenderOutcome>>,
}

impl HeadlessBrowser for FakeBrowser {
    fn render(&self, _url: &str) -> Result<RenderedPage, FetchError> {
        // Pop under the lock, then drop the guard before matching so a
        // `Panic` outcome cannot poison the mutex.
        let outcome = {
            let mut queue = self.behaviors.lock().expect("fake browser queue");
            queue
                .pop()
                .expect("FakeBrowser: render called more times than scripted")
        };
        match outcome {
            RenderOutcome::Ok(page) => Ok(page),
            RenderOutcome::Crash => Err(FetchError::BrowserCrash),
            RenderOutcome::RenderError(msg) => Err(FetchError::Render(msg)),
            RenderOutcome::Panic(msg) => panic!("{msg}"),
        }
    }

    fn name(&self) -> &'static str {
        "FakeBrowser"
    }
}

/// A fake factory that hands out a scripted sequence of browsers.
///
/// The Nth [`launch`](HeadlessBrowserFactory::launch) yields the Nth scripted
/// browser (LIFO via pop). Launch count is observable for thundering-herd tests.
pub(crate) struct FakeFactory {
    browsers: Mutex<Vec<Vec<RenderOutcome>>>,
    launch_count: AtomicUsize,
}

impl FakeFactory {
    /// Each entry is the behavior queue for one launched browser; the first
    /// launch pops index 0, the second pops index 1, etc.
    fn new(browsers: Vec<Vec<RenderOutcome>>) -> Arc<Self> {
        Arc::new(Self {
            browsers: Mutex::new(browsers),
            launch_count: AtomicUsize::new(0),
        })
    }

    fn launch_count(&self) -> usize {
        self.launch_count.load(Ordering::SeqCst)
    }

    /// Coerces to a trait-object factory for use as a fetcher/fetch_once arg.
    fn as_backend(self: &Arc<Self>) -> Arc<dyn HeadlessBrowserFactory> {
        self.clone()
    }
}

impl HeadlessBrowserFactory for FakeFactory {
    fn launch(&self) -> Result<Arc<dyn HeadlessBrowser>, FetchError> {
        self.launch_count.fetch_add(1, Ordering::SeqCst);
        let behaviors = {
            let queue = self.browsers.lock().expect("fake factory queue");
            queue
                .first()
                .expect("FakeFactory: launch called more times than scripted")
                .clone()
        };
        {
            let mut queue = self.browsers.lock().expect("fake factory queue");
            queue.remove(0);
        }
        Ok(Arc::new(FakeBrowser {
            behaviors: Mutex::new(behaviors),
        }))
    }

    fn name(&self) -> &'static str {
        "FakeFactory"
    }
}

// ---------------------------------------------------------------------------
// shared fixtures
// ---------------------------------------------------------------------------

fn ok_page(html: &str, url: &str) -> RenderedPage {
    RenderedPage {
        html: html.to_owned(),
        final_url: url.to_owned(),
    }
}

type ExtractorMap = std::collections::HashMap<OutputFormat, Arc<dyn Extractor>>;

fn empty_extractors() -> ExtractorMap {
    ExtractorMap::new()
}

fn html_options() -> FetchOptions {
    FetchOptions {
        format: OutputFormat::Html,
    }
}

type Slot = Arc<ParkingMutex<Option<Arc<dyn HeadlessBrowser>>>>;

fn fresh_slot() -> Slot {
    Arc::new(ParkingMutex::new(None))
}

// ===========================================================================
// classify_render_error
// ===========================================================================

#[test]
fn classify_connection_closed_marker_yields_browser_crash() {
    // Given the exact headless_chrome ConnectionClosed message.
    let display = "Unable to make method calls because underlying connection is closed";

    // When classifying.
    let err = classify_render_error(display);

    // Then it is a connection-level crash (triggers eviction).
    assert!(matches!(err, FetchError::BrowserCrash));
}

#[test]
fn classify_unrelated_render_failure_yields_render() {
    // Given an unrelated tab-level failure message.
    let display = "navigation timed out after 30000ms";

    // When classifying.
    let err = classify_render_error(display);

    // Then it is a per-tab render error (no eviction).
    assert!(matches!(err, FetchError::Render(_)));
}

// ===========================================================================
// classify_browser_error — type-safe downcast (primary detection path)
// ===========================================================================

#[test]
fn classify_real_connection_closed_type_yields_browser_crash() {
    // Given a genuine headless_chrome ConnectionClosed error, wrapped the same
    // way headless_chrome returns it from Transport::call_method (`.into()`
    // into anyhow::Error).
    let real = headless_chrome::browser::transport::ConnectionClosed {}.into();

    // When classifying.
    let err = classify_browser_error(&real);

    // Then it is a connection-level crash — independent of the Display text,
    // so a future headless_chrome release that reworded the message would
    // still be detected correctly.
    assert!(matches!(err, FetchError::BrowserCrash));
}

#[test]
fn classify_arbitrary_anyhow_error_yields_render() {
    // Given an unrelated anyhow error carrying no ConnectionClosed in its
    // source chain and a non-matching display string.
    let unrelated = anyhow::anyhow!("navigation timed out after 30000ms");

    // When classifying.
    let err = classify_browser_error(&unrelated);

    // Then it is a per-tab render error (no eviction).
    assert!(matches!(err, FetchError::Render(_)));
}

#[test]
fn classify_wrapped_connection_closed_in_source_chain_yields_browser_crash() {
    // Given an error that has ConnectionClosed in its anyhow chain, e.g.
    // headless_chrome context-wraps it via `.context(...)`.
    let wrapped = anyhow::Error::new(headless_chrome::browser::transport::ConnectionClosed {})
        .context("while opening new tab");

    // When classifying.
    let err = classify_browser_error(&wrapped);

    // Then the connection death is still detected through the chain.
    assert!(matches!(err, FetchError::BrowserCrash));
}

// ===========================================================================
// build_launch_options
// ===========================================================================

#[test]
fn launch_options_uses_ten_minute_idle_timeout() {
    // Given the production launch options builder and default stealth settings.
    let settings = StealthSettings::default();
    // When building.
    let opts = build_launch_options(&settings);

    #[expect(
        clippy::duration_suboptimal_units,
        reason = "`Duration::from_mins` is unstable; expressed in seconds"
    )]
    let expected = Duration::from_secs(600);
    assert_eq!(opts.idle_browser_timeout, expected);
}

#[test]
fn launch_options_includes_stealth_args_when_enabled() {
    // Given stealth enabled.
    let settings = StealthSettings::default();

    // When building.
    let opts = build_launch_options(&settings);

    // Then the stealth suppressor flags are present.
    let args: Vec<String> = opts
        .args
        .iter()
        .map(|s| s.to_string_lossy().into_owned())
        .collect();
    assert!(
        args.iter()
            .any(|a| a == "--disable-blink-features=AutomationControlled")
    );
}

#[test]
fn launch_options_omits_stealth_args_when_disabled() {
    // Given stealth disabled.
    let settings = StealthSettings {
        enabled: false,
        ..StealthSettings::default()
    };

    // When building.
    let opts = build_launch_options(&settings);

    // Then no stealth flags are added.
    let args: Vec<String> = opts
        .args
        .iter()
        .map(|s| s.to_string_lossy().into_owned())
        .collect();
    assert!(
        !args
            .iter()
            .any(|a| a == "--disable-blink-features=AutomationControlled")
    );
}

#[test]
fn launch_options_uses_configured_binary_path() {
    // Given a configured binary path.
    let mut settings = StealthSettings::default();
    let custom = std::path::PathBuf::from("/usr/bin/google-chrome");
    settings.binary_path = Some(custom.clone());

    // When building.
    let opts = build_launch_options(&settings);

    // Then the binary path is set on LaunchOptions.
    assert_eq!(opts.path.as_ref(), Some(&custom));
}

// ===========================================================================
// extract_content
// ===========================================================================

#[test]
fn extract_html_format_passes_through_without_extractor() {
    // Given rendered HTML and no registered extractor.
    // When extracting for the Html format.
    let out = extract_content("<p>hi</p>", &html_options(), &empty_extractors());

    // Then the raw HTML is returned unchanged.
    assert_eq!(out, "<p>hi</p>");
}

// ===========================================================================
// fetch_once: launch / eviction / per-tab retention
// ===========================================================================

#[test]
fn fetch_once_succeeds_on_first_browser() {
    // Given an empty slot and a factory that launches a working browser.
    let factory = FakeFactory::new(vec![vec![RenderOutcome::Ok(ok_page(
        "<p>hi</p>",
        "https://example.com/",
    ))]]);
    let slot = fresh_slot();

    // When running one fetch attempt.
    let result = fetch_once(
        &slot,
        &factory.as_backend(),
        "https://example.com/",
        &html_options(),
        &empty_extractors(),
    );

    // Then the fetch succeeds with the rendered content.
    assert_eq!(result.unwrap().content, "<p>hi</p>");
}

#[test]
fn fetch_once_evicts_browser_on_connection_death() {
    // Given a factory whose browser dies with ConnectionClosed.
    let factory = FakeFactory::new(vec![vec![RenderOutcome::Crash]]);
    let slot = fresh_slot();

    // When running one fetch attempt.
    let _ = fetch_once(
        &slot,
        &factory.as_backend(),
        "https://example.com/",
        &html_options(),
        &empty_extractors(),
    );

    // Then the slot is evicted (ready for a relaunch).
    assert!(
        slot.lock().is_none(),
        "slot should be evicted after a BrowserCrash"
    );
}

#[test]
fn fetch_once_keeps_browser_on_per_tab_render_error() {
    // Given a factory whose browser returns a per-tab render error.
    let factory = FakeFactory::new(vec![vec![RenderOutcome::RenderError(
        "navigation timed out".to_owned(),
    )]]);
    let slot = fresh_slot();

    // When running one fetch attempt.
    let _ = fetch_once(
        &slot,
        &factory.as_backend(),
        "https://example.com/",
        &html_options(),
        &empty_extractors(),
    );

    // Then the browser is NOT evicted (per-tab failures must not kill the
    // shared handle under concurrency).
    assert!(
        slot.lock().is_some(),
        "slot must be retained after a per-tab Render error"
    );
}

// ===========================================================================
// fetch (full retry state machine via spawn_blocking)
// ===========================================================================

/// Builds a fetcher and returns it alongside its concrete factory so tests can
/// assert on launch count.
fn fetcher_and_factory(
    browsers: Vec<Vec<RenderOutcome>>,
) -> (super::HeadlessChromeFetcher, Arc<FakeFactory>) {
    let factory = FakeFactory::new(browsers);
    let fetcher =
        super::HeadlessChromeFetcher::with_factory(empty_extractors(), factory.as_backend());
    (fetcher, factory)
}

#[tokio::test]
async fn first_fetch_succeeds_via_cached_browser() {
    // Given a fetcher with a working browser.
    let (fetcher, factory) = fetcher_and_factory(vec![vec![RenderOutcome::Ok(ok_page(
        "<p>hi</p>",
        "https://example.com/",
    ))]]);

    // When fetching.
    let out = fetcher
        .fetch("https://example.com/", html_options())
        .await
        .unwrap();

    // Then the rendered content is returned and the browser launched exactly once.
    assert_eq!(out.content, "<p>hi</p>");
    assert_eq!(factory.launch_count(), 1);
}

#[tokio::test]
async fn fetch_retries_once_and_succeeds_after_connection_death() {
    // Given a fetcher whose first browser dies, then a second launch works
    // (simulates idle-teardown recovery).
    let (fetcher, factory) = fetcher_and_factory(vec![
        vec![RenderOutcome::Crash],
        vec![RenderOutcome::Ok(ok_page(
            "<p>recovered</p>",
            "https://example.com/",
        ))],
    ]);

    // When fetching.
    let out = fetcher
        .fetch("https://example.com/", html_options())
        .await
        .unwrap();

    // Then the content is recovered and the browser launched twice (crash + relaunch).
    assert_eq!(out.content, "<p>recovered</p>");
    assert_eq!(factory.launch_count(), 2);
}

#[tokio::test]
async fn fetch_returns_render_error_without_retry() {
    // Given a fetcher whose browser returns a per-tab render error.
    let (fetcher, factory) = fetcher_and_factory(vec![vec![RenderOutcome::RenderError(
        "navigation timed out".to_owned(),
    )]]);

    // When fetching.
    let err = fetcher
        .fetch("https://example.com/", html_options())
        .await
        .unwrap_err();

    // Then the error is a per-tab Render (not a crash) and only one launch happened.
    assert!(matches!(err, FetchError::Render(_)));
    assert_eq!(factory.launch_count(), 1);
}

#[tokio::test]
async fn fetch_does_not_retry_more_than_once_on_repeated_crash() {
    // Given a fetcher where both the first and second browser die on first render.
    let (fetcher, factory) =
        fetcher_and_factory(vec![vec![RenderOutcome::Crash], vec![RenderOutcome::Crash]]);

    // When fetching.
    let err = fetcher
        .fetch("https://example.com/", html_options())
        .await
        .unwrap_err();

    // Then the error surfaces (no infinite loop) and exactly two launches happened.
    assert!(matches!(err, FetchError::BrowserCrash));
    assert_eq!(factory.launch_count(), 2);
}

#[tokio::test]
async fn fetch_recovers_from_panic_inside_blocking_task() {
    // Given a fetcher whose first browser panics; a second launch would work,
    // but the retry path only triggers on BrowserCrash, so the JoinError from
    // the panic maps to a Render error instead.
    let (fetcher, _factory) = fetcher_and_factory(vec![
        vec![RenderOutcome::Panic("simulated chrome panic".to_owned())],
        vec![RenderOutcome::Ok(ok_page(
            "<p>recovered</p>",
            "https://example.com/",
        ))],
    ]);

    // When fetching.
    let result = fetcher.fetch("https://example.com/", html_options()).await;

    // Then the panic does not propagate out of fetch.
    assert!(matches!(result, Err(FetchError::Render(_))));
}

// ===========================================================================
// Concurrency: no thundering herd of relaunches
// ===========================================================================

#[tokio::test]
async fn concurrent_crashes_trigger_single_relunch() {
    // Given a fetcher whose first browser dies on two renders and a second
    // launch yields a browser that can serve two renders. Two concurrent
    // fetches share the slot via Arc<dyn WebFetcher>.
    let (fetcher, factory) = fetcher_and_factory(vec![
        vec![RenderOutcome::Crash, RenderOutcome::Crash],
        vec![
            RenderOutcome::Ok(ok_page("<p>a</p>", "https://a.example.com/")),
            RenderOutcome::Ok(ok_page("<p>b</p>", "https://b.example.com/")),
        ],
    ]);
    let shared: Arc<dyn WebFetcher> = Arc::new(fetcher);

    // When two fetches run concurrently (each gets its own clone of the handle).
    let a = shared.clone();
    let b = shared.clone();
    let (ra, rb) = tokio::join!(
        tokio::spawn(async move { a.fetch("https://a.example.com/", html_options()).await }),
        tokio::spawn(async move { b.fetch("https://b.example.com/", html_options()).await }),
    );

    // Then both fetches complete and the browser launched exactly twice
    // (one initial, one recovery) — the slot mutex serializes relaunch so
    // there is no thundering herd.
    ra.unwrap().unwrap();
    rb.unwrap().unwrap();
    assert_eq!(factory.launch_count(), 2);
}
