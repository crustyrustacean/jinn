//! Unit tests for the headless Chrome fetcher and the shared browser.
//!
//! The fetcher is a thin adapter; the interesting behavior (launch, eviction,
//! retry) lives in [`SharedBrowser`]. Both are tested via a fake
//! [`HeadlessBrowserFactory`] that scripts a sequence of browser behaviors
//! (success, connection death, render error). This drives the launch /
//! eviction / retry state machine without spawning a real Chromium.
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

use crate::shared_browser::{
    HeadlessBrowser, HeadlessBrowserFactory, RenderedPage, SharedBrowser, build_launch_options,
    classify_browser_error, classify_render_error,
};
use crate::stealth::StealthSettings;
use crate::{Extractor, FetchError, FetchOptions, OutputFormat, WebFetcher};

// ---------------------------------------------------------------------------
// Test fakes
// ---------------------------------------------------------------------------

/// The scripted behavior for one fake browser.
///
/// Carries a queue of render outcomes (one per `render` call) and a queue
/// of liveness outcomes (one per `liveness` call). Both default to
/// empty/`Ok` so existing tests that only script renders stay unchanged.
///
/// Each queue is drained LIFO: a call pops the tail; when empty, `render`
/// panics (render count is always asserted by the test's scripted queue)
/// while `liveness` returns `Ok` (a healthy browser is the common case).
#[derive(Clone, Default)]
struct FakeBrowserScript {
    behaviors: Vec<RenderOutcome>,
    liveness: Vec<LivenessOutcome>,
}

impl From<Vec<RenderOutcome>> for FakeBrowserScript {
    fn from(behaviors: Vec<RenderOutcome>) -> Self {
        Self {
            behaviors,
            liveness: Vec::new(),
        }
    }
}

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

/// A liveness outcome the fake browser should produce for one `liveness` call.
///
/// The default (empty queue) is `Ok`, so a healthy browser needs no scripting.
#[derive(Clone)]
enum LivenessOutcome {
    /// Probe succeeds.
    Ok,
    /// Connection-level death (maps to `BrowserCrash`).
    Crash,
}

/// A fake browser that serves queued outcomes per call, LIFO.
struct FakeBrowser {
    behaviors: Mutex<Vec<RenderOutcome>>,
    liveness: Mutex<Vec<LivenessOutcome>>,
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

    fn liveness(&self) -> Result<(), FetchError> {
        // Pop under the lock, then drop the guard before matching so a
        // `Panic` outcome cannot poison the mutex. An empty queue defaults
        // to Ok: a healthy browser is the common case, so render-only tests
        // never need to script liveness.
        let outcome = {
            let mut queue = self.liveness.lock().expect("fake browser liveness queue");
            queue.pop().unwrap_or(LivenessOutcome::Ok)
        };
        match outcome {
            LivenessOutcome::Ok => Ok(()),
            LivenessOutcome::Crash => Err(FetchError::BrowserCrash),
        }
    }

    fn name(&self) -> &'static str {
        "FakeBrowser"
    }
}

/// A fake factory that hands out a scripted sequence of browsers.
///
/// The Nth [`launch`](HeadlessBrowserFactory::launch) yields the Nth scripted
/// browser (FIFO). Launch count is observable for thundering-herd tests.
///
/// Each script is a [`FakeBrowserScript`]; [`FakeFactory::new`] accepts
/// `Vec<Vec<RenderOutcome>>` via the `From` impl so render-only tests stay terse.
pub(crate) struct FakeFactory {
    browsers: Mutex<Vec<FakeBrowserScript>>,
    launch_count: AtomicUsize,
}

impl FakeFactory {
    /// Each entry scripts one launched browser; the first launch pops index 0,
    /// the second pops index 1, etc.
    fn new<S: Into<FakeBrowserScript>>(browsers: Vec<S>) -> Arc<Self> {
        Arc::new(Self {
            browsers: Mutex::new(browsers.into_iter().map(Into::into).collect()),
            launch_count: AtomicUsize::new(0),
        })
    }

    fn launch_count(&self) -> usize {
        self.launch_count.load(Ordering::SeqCst)
    }

    /// Coerces to a trait-object factory for use as a shared browser arg.
    fn as_backend(self: &Arc<Self>) -> Arc<dyn HeadlessBrowserFactory> {
        self.clone()
    }
}

impl HeadlessBrowserFactory for FakeFactory {
    fn launch(&self) -> Result<Arc<dyn HeadlessBrowser>, FetchError> {
        self.launch_count.fetch_add(1, Ordering::SeqCst);
        let script = {
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
            behaviors: Mutex::new(script.behaviors),
            liveness: Mutex::new(script.liveness),
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
// build_launch_options — headless/headed mode flags
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
    let expected = std::time::Duration::from_secs(600);
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

#[test]
fn launch_options_runs_headless_by_default() {
    // Given default (headless) stealth settings.
    let settings = StealthSettings::default();

    // When building.
    let opts = build_launch_options(&settings);

    // Then headless is true and no persistent profile dir is set.
    assert!(opts.headless);
    assert!(opts.user_data_dir.is_none());
}

#[test]
fn launch_options_runs_headed_when_headed_set() {
    // Given headed stealth settings.
    let settings = StealthSettings {
        headed: true,
        ..StealthSettings::default()
    };

    // When building.
    let opts = build_launch_options(&settings);

    // Then headless is false.
    assert!(!opts.headless);
}

#[test]
fn launch_options_sets_user_data_dir_when_profile_given() {
    // Given a persistent profile dir.
    let dir = std::path::PathBuf::from("/tmp/jinn-profile/headed");
    let settings = StealthSettings {
        profile_dir: Some(dir.clone()),
        ..StealthSettings::default()
    };

    // When building.
    let opts = build_launch_options(&settings);

    // Then the user data dir matches the configured profile.
    assert_eq!(opts.user_data_dir.as_ref(), Some(&dir));
}

// ===========================================================================
// SharedBrowser: launch / eviction / per-tab retention
// ===========================================================================

/// Builds a fetcher backed by a `SharedBrowser` with a fake factory, returning
/// both so tests can assert on launch count.
fn fetcher_and_factory(
    browsers: Vec<Vec<RenderOutcome>>,
) -> (super::HeadlessChromeFetcher, Arc<FakeFactory>) {
    let factory = FakeFactory::new(browsers);
    let shared = Arc::new(SharedBrowser::with_factory(factory.as_backend()));
    let fetcher = super::HeadlessChromeFetcher::with_shared(shared, empty_extractors());
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

// ===========================================================================
// Shared sharing: two fetchers on the same SharedBrowser launch once
// ===========================================================================

#[tokio::test]
async fn two_fetchers_sharing_one_browser_launch_once() {
    // Given a single SharedBrowser with a factory that can serve two renders,
    // shared between two fetchers (simulates fetch + search on one mode).
    let factory = FakeFactory::new(vec![vec![
        RenderOutcome::Ok(ok_page("<p>a</p>", "https://a.example.com/")),
        RenderOutcome::Ok(ok_page("<p>b</p>", "https://b.example.com/")),
    ]]);
    let shared = Arc::new(SharedBrowser::with_factory(factory.as_backend()));
    let fetcher_a = super::HeadlessChromeFetcher::with_shared(shared.clone(), empty_extractors());
    let fetcher_b = super::HeadlessChromeFetcher::with_shared(shared, empty_extractors());

    // When both fetchers fetch.
    let (a, b) = tokio::join!(
        fetcher_a.fetch("https://a.example.com/", html_options()),
        fetcher_b.fetch("https://b.example.com/", html_options()),
    );

    // Then both succeed (with one of the scripted contents) and the browser
    // launched exactly once — proving the two tools share one process when
    // given the same SharedBrowser. The two scripted pages are identical in
    // shape, so we only assert each got a valid `<p>` block.
    let a_content = a.unwrap().content;
    let b_content = b.unwrap().content;
    assert!(a_content.starts_with("<p>") && a_content.ends_with("</p>"));
    assert!(b_content.starts_with("<p>") && b_content.ends_with("</p>"));
    assert_eq!(factory.launch_count(), 1);
}

// ===========================================================================
// SharedBrowser: heartbeat probe / force_evict
// ===========================================================================

/// Builds a `SharedBrowser` with a fake factory, returning both so the
/// probe/force-evict tests can assert on launch count. The browser is NOT
/// launched until something warms the slot (a render).
fn shared_and_factory<S: Into<FakeBrowserScript>>(browsers: Vec<S>) -> (Arc<SharedBrowser>, Arc<FakeFactory>) {
    let factory = FakeFactory::new(browsers);
    let shared = Arc::new(SharedBrowser::with_factory(factory.as_backend()));
    (shared, factory)
}

#[test]
fn probe_is_noop_when_slot_empty() {
    // Given a shared browser with a factory, never warmed (slot empty).
    let (shared, factory) = shared_and_factory(vec![vec![RenderOutcome::Ok(ok_page("<p>hi</p>", "https://example.com/"))]]);

    // When probing.
    shared.probe();

    // Then no browser was launched — the heartbeat never creates a browser.
    assert_eq!(factory.launch_count(), 0);
}

#[test]
fn probe_keeps_healthy_browser() {
    // Given a warmed browser whose liveness is Ok (the default).
    let (shared, factory) = shared_and_factory(vec![vec![
        RenderOutcome::Ok(ok_page("<p>hi</p>", "https://example.com/")),
        // A second Ok in case a second render is needed to assert reuse.
        RenderOutcome::Ok(ok_page("<p>hi</p>", "https://example.com/")),
    ]]);
    shared.render_page("https://example.com/").expect("warm render");
    assert_eq!(factory.launch_count(), 1);

    // When probing.
    shared.probe();

    // Then the browser is reused on the next render — the probe did NOT evict.
    shared.render_page("https://example.com/").expect("reuse render");
    assert_eq!(factory.launch_count(), 1);
}


#[test]
fn force_evict_is_noop_on_empty_slot() {
    // Given a shared browser with an empty slot.
    let (shared, factory) = shared_and_factory(vec![vec![RenderOutcome::Ok(ok_page("<p>hi</p>", "https://example.com/"))]]);

    // When force-evicting twice on an already-empty slot.
    shared.force_evict();
    shared.force_evict();

    // Then no panic, and nothing launched.
    assert_eq!(factory.launch_count(), 0);
}

#[test]
fn next_render_relaunches_after_probe_eviction() {
    // Given browser #1 (liveness Crash) and browser #2 (render Ok) so the
    // post-eviction render can succeed against a fresh browser.
    let mut first: FakeBrowserScript = vec![RenderOutcome::Ok(ok_page("<p>one</p>", "https://example.com/"))].into();
    first.liveness = vec![LivenessOutcome::Crash];
    let second: FakeBrowserScript = vec![RenderOutcome::Ok(ok_page("<p>two</p>", "https://example.com/"))].into();
    let (shared, factory) = shared_and_factory(vec![first, second]);
    shared.render_page("https://example.com/").expect("warm render #1");
    assert_eq!(factory.launch_count(), 1);

    // When the probe evicts, then a render runs.
    shared.probe();
    let out = shared.render_page("https://example.com/").expect("render after eviction");

    // Then the render succeeded against browser #2 (launch_count -> 2).
    assert_eq!(out.html, "<p>two</p>");
    assert_eq!(factory.launch_count(), 2);
}
