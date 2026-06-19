//! Headless Chrome fetcher - fetches JS-rendered pages via Chromium.
//!
//! Uses [`headless_chrome::Browser`] to launch a headless Chromium process on
//! first use (lazy launch), reuses it across requests, and cleanly shuts it
//! down when the actor system stops.
//!
//! # Crash recovery
//!
//! If a tab operation fails with a connection-level death (idle-teardown,
//! browser crash, OOM kill), the browser handle is cleared from the internal
//! [`Mutex`] and the fetch is retried exactly once against a freshly-launched
//! browser. Per-tab failures (a bad page, a tab-level timeout) do _not_ evict
//! the shared browser, since under concurrency that would kill other sessions'
//! in-flight tabs.
//!
//! # Lifecycle
//!
//! - **Lazy launch**: first `fetch()` starts Chromium.
//! - **Reuse**: subsequent calls open a new tab on the same browser.
//! - **Self-heal**: a dead WebSocket triggers exactly one relaunch + retry.
//! - **Shutdown**: [`WebFetcher::shutdown`] drops the browser (kills process).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use headless_chrome::{Browser, LaunchOptions};
use parking_lot::Mutex;

use crate::{Extractor, FetchError, FetchOptions, FetchOutput, OutputFormat, WebFetcher};

/// How long a kept-warm browser survives while idle before headless_chrome
/// tears its WebSocket down. The library default (30s) is far too eager;
/// self-heal still recovers when a genuine death eventually occurs past this.
/// 10 minutes — long enough to avoid churn on natural idle gaps while
// still tearing down an unused browser before it lingers indefinitely.
// `Duration::from_mins` is unstable, so we express the constant in seconds.
#[expect(
    clippy::duration_suboptimal_units,
    reason = "`Duration::from_mins` is unstable; expressed in seconds"
)]
const IDLE_BROWSER_TIMEOUT: Duration = Duration::from_secs(600);

/// A page rendered to HTML by a headless browser tab.
///
/// Extraction (text/markdown) is the fetcher's concern, applied after render,
/// so the browser abstraction stays format-agnostic.
#[derive(Clone)]
pub(crate) struct RenderedPage {
    /// Raw HTML after JavaScript execution.
    html: String,
    /// Final URL after any redirects.
    final_url: String,
}

/// Capability: render one page to HTML in a headless browser tab.
///
/// Abstracts the concrete [`headless_chrome::Browser`] so the fetcher's launch,
/// eviction, and retry logic is unit-testable without spawning Chromium.
/// Implementations classify their own errors: connection death surfaces as
/// [`FetchError::BrowserCrash`]; per-tab failures as [`FetchError::Render`].
pub(crate) trait HeadlessBrowser: Send + Sync {
    /// Renders `url` to a page.
    ///
    /// # Errors
    ///
    /// [`FetchError::BrowserCrash`] when the shared connection is dead;
    /// [`FetchError::Render`] for per-tab failures.
    fn render(&self, url: &str) -> Result<RenderedPage, FetchError>;
    /// Backend identifier for tracing/debug.
    fn name(&self) -> &'static str;
}

/// Capability: launch a fresh headless browser handle.
///
/// The fetcher calls this on first use and on every crash-recovery relaunch.
/// Under concurrency the slot mutex serializes relaunches, so only one task
/// actually launches and the rest reuse it.
pub(crate) trait HeadlessBrowserFactory: Send + Sync {
    /// Launches a new browser.
    ///
    /// # Errors
    ///
    /// [`FetchError::BrowserLaunch`] if the process cannot be started.
    fn launch(&self) -> Result<Arc<dyn HeadlessBrowser>, FetchError>;
    /// Factory identifier for tracing/debug.
    fn name(&self) -> &'static str;
}

/// The [`LaunchOptions`] used for every Chromium launch.
///
/// Exposed `pub(crate)` so the idle-timeout invariant is unit-testable.
pub(crate) fn build_launch_options() -> LaunchOptions<'static> {
    LaunchOptions {
        headless: true,
        idle_browser_timeout: IDLE_BROWSER_TIMEOUT,
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Concrete headless_chrome backend
// ---------------------------------------------------------------------------

/// Production backend: wraps a real [`headless_chrome::Browser`].
struct ChromeBrowser {
    browser: Browser,
}

impl HeadlessBrowser for ChromeBrowser {
    fn render(&self, url: &str) -> Result<RenderedPage, FetchError> {
        let tab = self
            .browser
            .new_tab()
            .map_err(|e| classify_render_error(&e.to_string()))?;

        tracing::trace!(url = %url, "HeadlessChromeFetcher: navigating to URL");
        tab.navigate_to(url)
            .map_err(|e| classify_render_error(&e.to_string()))?
            .wait_until_navigated()
            .map_err(|e| classify_render_error(&e.to_string()))?;
        tracing::trace!("HeadlessChromeFetcher: navigation complete");

        tracing::trace!("HeadlessChromeFetcher: getting page HTML");
        let html = tab
            .get_content()
            .map_err(|e| classify_render_error(&e.to_string()))?;
        tracing::debug!(
            html_len = html.len(),
            "HeadlessChromeFetcher: HTML retrieved"
        );

        let final_url = tab.get_url();
        tracing::debug!(final_url = %final_url, "HeadlessChromeFetcher: final URL");

        tracing::trace!("HeadlessChromeFetcher: closing tab");
        let _ = tab.close(true);

        Ok(RenderedPage { html, final_url })
    }

    fn name(&self) -> &'static str {
        "headless_chrome::Browser"
    }
}

/// Production factory: launches a real Chromium via [`headless_chrome`].
struct ChromeFactory;

impl HeadlessBrowserFactory for ChromeFactory {
    fn launch(&self) -> Result<Arc<dyn HeadlessBrowser>, FetchError> {
        tracing::info!("HeadlessChromeFetcher: launching headless Chrome");
        let browser = Browser::new(build_launch_options()).map_err(|e| {
            tracing::error!(err = %e, "HeadlessChromeFetcher: failed to launch browser");
            FetchError::BrowserLaunch
        })?;
        tracing::info!("HeadlessChromeFetcher: browser launched successfully");
        Ok(Arc::new(ChromeBrowser { browser }))
    }

    fn name(&self) -> &'static str {
        "ChromeFactory"
    }
}

/// The literal text of the headless_chrome `ConnectionClosed` error.
///
/// Used to detect a dead WebSocket without depending on a downcast into
/// `headless_chrome`'s (transitively-available) `anyhow` error tree. See
/// the crate's `src/browser/transport/mod.rs`: the error is
/// `#[error("Unable to make method calls because underlying connection is closed")]`.
const CONNECTION_CLOSED_MARKER: &str = "underlying connection is closed";

/// Maps a headless_chrome failure string to a [`FetchError`].
///
/// Detects `ConnectionClosed` (the shared WebSocket died: idle-teardown
/// timeout, OOM kill, real crash) via its error message and maps it to
/// [`FetchError::BrowserCrash`] so the caller can evict the shared browser
/// and relaunch. All other failures (a bad page, a tab-level timeout) stay
/// as [`FetchError::Render`], since they must not evict the shared browser
/// under concurrency.
fn classify_render_error(display: &str) -> FetchError {
    if display.contains(CONNECTION_CLOSED_MARKER) {
        FetchError::BrowserCrash
    } else {
        FetchError::Render(display.to_owned())
    }
}

// ---------------------------------------------------------------------------
// Fetcher
// ---------------------------------------------------------------------------

/// A web fetcher that uses headless Chrome to render JavaScript-heavy pages.
///
/// The browser is lazily launched on the first `fetch()` call and reused
/// across subsequent calls. Thread-safe via a shared slot guarded by a mutex,
/// held only during launch/clone/evict — never across a render — so concurrent
/// fetches run independent tabs.
///
/// Content extraction is delegated to [`Extractor`] implementations looked
/// up by [`OutputFormat`]. Formats without a registered extractor (e.g.,
/// [`OutputFormat::Html`]) return the raw page HTML unchanged.
pub struct HeadlessChromeFetcher {
    /// The lazily-launched browser instance.
    browser: Arc<Mutex<Option<Arc<dyn HeadlessBrowser>>>>,
    /// Extractor implementations keyed by output format.
    /// Formats not in the map (e.g., `Html`) pass through raw content.
    extractors: HashMap<OutputFormat, Arc<dyn Extractor>>,
    /// Produces new browser handles on first use and crash recovery.
    factory: Arc<dyn HeadlessBrowserFactory>,
}

impl HeadlessChromeFetcher {
    /// Creates a new fetcher with the given extractor map, without launching a browser.
    ///
    /// The browser will be launched on the first `fetch()` call.
    #[must_use]
    pub fn new(extractors: HashMap<OutputFormat, Arc<dyn Extractor>>) -> Self {
        Self::with_factory(extractors, Arc::new(ChromeFactory))
    }

    /// Test seam: creates a fetcher backed by a swappable browser factory.
    ///
    /// Production code uses [`Self::new`] (the real Chromium factory). Tests
    /// inject a fake factory to drive self-heal, retry, and eviction behavior
    /// without spawning Chrome.
    /// Constructs a fetcher with a specific browser factory.
    ///
    /// `pub(crate)` so tests can inject a fake factory; production uses
    /// [`new`](Self::new), which wires the real [`ChromeFactory`].
    pub(crate) fn with_factory(
        extractors: HashMap<OutputFormat, Arc<dyn Extractor>>,
        factory: Arc<dyn HeadlessBrowserFactory>,
    ) -> Self {
        Self {
            browser: Arc::new(Mutex::new(None)),
            extractors,
            factory,
        }
    }
}

/// Ensures a browser is running in `slot`, launching one if necessary.
///
/// Returns a clone of the handle. The lock is held only long enough to
/// check/launch/clone — never across a render — so concurrent fetches can each
/// grab a handle and run independent tabs. Because launch happens under the
/// lock, concurrent crash-recovery attempts funnel to exactly one relaunch.
fn ensure_browser(
    slot: &Arc<Mutex<Option<Arc<dyn HeadlessBrowser>>>>,
    factory: &Arc<dyn HeadlessBrowserFactory>,
) -> Result<Arc<dyn HeadlessBrowser>, FetchError> {
    let mut guard = slot.lock();
    if let Some(ref browser) = *guard {
        tracing::trace!("HeadlessChromeFetcher: reusing existing browser");
        return Ok(browser.clone());
    }
    tracing::info!(factory = %factory.name(), "HeadlessChromeFetcher: launching headless browser");
    let browser = factory.launch()?;
    *guard = Some(browser.clone());
    Ok(browser)
}

/// Evicts the stored browser from `slot` **only if** it is the same handle as
/// `offender`. Returns the old handle when it evicted.
///
/// This guards against the concurrency race where task A crashes on browser 1,
/// relaunches browser 2, and then task B (still holding its dead clone of
/// browser 1) calls eviction — without the `ptr_eq` check B would evict A's
/// freshly-launched browser 2. Comparing the handle identity scopes eviction
/// to exactly the browser that died.
fn evict_if_matching(
    slot: &Arc<Mutex<Option<Arc<dyn HeadlessBrowser>>>>,
    offender: &Arc<dyn HeadlessBrowser>,
) -> bool {
    let mut guard = slot.lock();
    if guard
        .as_ref()
        .is_some_and(|current| Arc::ptr_eq(current, offender))
    {
        guard.take();
        true
    } else {
        false
    }
}

/// Applies the configured extractor (if any) to rendered HTML.
fn extract_content(
    html: &str,
    options: &FetchOptions,
    extractors: &HashMap<OutputFormat, Arc<dyn Extractor>>,
) -> String {
    tracing::trace!(format = ?options.format, "HeadlessChromeFetcher: extracting content");
    match extractors.get(&options.format) {
        Some(extractor) => extractor.extract(html),
        None => html.to_owned(),
    }
}

/// One fetch attempt against the cached browser: ensure a handle, render, extract.
///
/// On a connection-level death ([`FetchError::BrowserCrash`]), evicts the
/// shared handle so the next attempt relaunches. Per-tab failures
/// ([`FetchError::Render`]) are returned without eviction — evicting on them
/// would kill other sessions' in-flight tabs under concurrency.
fn fetch_once(
    browser_slot: &Arc<Mutex<Option<Arc<dyn HeadlessBrowser>>>>,
    factory: &Arc<dyn HeadlessBrowserFactory>,
    url: &str,
    options: &FetchOptions,
    extractors: &HashMap<OutputFormat, Arc<dyn Extractor>>,
) -> Result<FetchOutput, FetchError> {
    let browser = ensure_browser(browser_slot, factory)?;
    match browser.render(url) {
        Ok(page) => {
            let content = extract_content(&page.html, options, extractors);
            tracing::debug!(
                content_len = content.len(),
                "HeadlessChromeFetcher: content extracted"
            );
            Ok(FetchOutput {
                content,
                url: page.final_url,
                status: 200,
                content_type: "text/html".to_owned(),
            })
        }
        Err(err) => {
            tracing::warn!(err = %err, "HeadlessChromeFetcher: render failed");
            // Evict only on connection death, and only if the slot still holds
            // THIS task's handle — a concurrent task may have already relaunched.
            if matches!(err, FetchError::BrowserCrash | FetchError::BrowserLaunch)
                && evict_if_matching(browser_slot, &browser)
            {
                tracing::info!("HeadlessChromeFetcher: clearing browser for crash recovery");
            }
            Err(err)
        }
    }
}

#[async_trait]
impl WebFetcher for HeadlessChromeFetcher {
    async fn fetch(&self, url: &str, options: FetchOptions) -> Result<FetchOutput, FetchError> {
        tracing::debug!(url = %url, format = ?options.format, "HeadlessChromeFetcher: starting fetch");
        // Validate URL.
        let parsed = url::Url::parse(url).map_err(|e| FetchError::InvalidUrl(e.to_string()))?;
        match parsed.scheme() {
            "http" | "https" => {}
            other => {
                return Err(FetchError::InvalidUrl(format!(
                    "unsupported scheme: {other}"
                )));
            }
        }

        // headless_chrome tab ops busy-loop on thread::sleep (util::Wait::until),
        // so they must never run on a tokio worker thread. Run the whole fetch
        // (attempt + retry) on the blocking pool. The browser slot and factory
        // are shared via Arc, so the cached Chrome is still reused across calls.
        let browser_slot = self.browser.clone();
        let extractors = self.extractors.clone();
        let factory = self.factory.clone();
        let url_owned = url.to_owned();
        let join = tokio::task::spawn_blocking(move || {
            match fetch_once(&browser_slot, &factory, &url_owned, &options, &extractors) {
                Err(FetchError::BrowserCrash) => {
                    // Connection-level death: the shared WebSocket is gone.
                    // `fetch_once` already evicted the handle; relaunch and
                    // retry exactly once.
                    tracing::info!("HeadlessChromeFetcher: retrying after connection death");
                    fetch_once(&browser_slot, &factory, &url_owned, &options, &extractors)
                }
                other => other,
            }
        });
        // Map a panic inside the blocking task to a Render error rather than
        // propagating the JoinError; headless_chrome has panicking code paths.
        match join.await {
            Ok(inner) => inner,
            Err(_join_err) => Err(FetchError::Render("browser task panicked".to_owned())),
        }
    }

    async fn shutdown(&self) {
        tracing::info!("HeadlessChromeFetcher: shutting down");
        if let Some(browser) = self.browser.lock().take() {
            tracing::debug!(
                backend = browser.name(),
                "HeadlessChromeFetcher: dropping browser (kills Chromium process)"
            );
            // Drop the browser - this kills the Chromium process.
            drop(browser);
        }
        tracing::info!("HeadlessChromeFetcher: shutdown complete");
    }
}

impl Default for HeadlessChromeFetcher {
    fn default() -> Self {
        Self::new(HashMap::new())
    }
}

#[cfg(test)]
mod tests;
