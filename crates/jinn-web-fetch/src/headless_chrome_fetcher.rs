//! Headless Chrome fetcher - fetches JS-rendered pages via Chromium.
//!
//! Uses [`headless_chrome::Browser`] to launch a headless Chromium process on
//! first use (lazy launch), reuses it across requests, and cleanly shuts it
//! down when the actor system stops.
//!
//! # Crash recovery
//!
//! If a tab operation fails (browser crash, OOM kill, etc.), the browser
//! instance is cleared from the internal [`Mutex`]. The next `fetch()` call
//! will re-launch. The caller receives [`FetchError::BrowserCrash`].
//!
//! # Lifecycle
//!
//! - **Lazy launch**: first `fetch()` starts Chromium.
//! - **Reuse**: subsequent calls open a new tab on the same browser.
//! - **Shutdown**: [`WebFetcher::shutdown`] drops the browser (kills process).

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;

use async_trait::async_trait;
use headless_chrome::{Browser, LaunchOptions};
use std::time::Duration;
use tracing;

use crate::{Extractor, FetchError, FetchOptions, FetchOutput, OutputFormat, WebFetcher};

/// A web fetcher that uses headless Chrome to render JavaScript-heavy pages.
///
/// The browser is lazily launched on the first `fetch()` call and reused
/// across subsequent calls. Thread-safe via `Arc<Mutex<Option<Browser>>>`.
///
/// Content extraction is delegated to [`Extractor`] implementations looked
/// up by [`OutputFormat`]. Formats without a registered extractor (e.g.,
/// [`OutputFormat::Html`]) return the raw page HTML unchanged.
pub struct HeadlessChromeFetcher {
    /// The lazily-launched browser instance.
    browser: Arc<Mutex<Option<Browser>>>,
    /// Extractor implementations keyed by output format.
    /// Formats not in the map (e.g., `Html`) pass through raw content.
    extractors: HashMap<OutputFormat, Arc<dyn Extractor>>,
}

impl HeadlessChromeFetcher {
    /// Creates a new fetcher with the given extractor map, without launching a browser.
    ///
    /// The browser will be launched on the first `fetch()` call.
    #[must_use]
    pub fn new(extractors: HashMap<OutputFormat, Arc<dyn Extractor>>) -> Self {
        Self {
            browser: Arc::new(Mutex::new(None)),
            extractors,
        }
    }

}

/// Ensures a browser is running in `slot`, launching one if necessary.
///
/// Returns a clone of the handle. The lock is held only long enough to
/// check/launch/clone — never across tab operations — so concurrent fetches
/// can each grab a handle and run independent tabs.
fn ensure_browser(slot: &Arc<Mutex<Option<Browser>>>) -> Result<Browser, FetchError> {
    let mut guard = slot.lock();
    if let Some(ref browser) = *guard {
        tracing::trace!("HeadlessChromeFetcher: reusing existing browser");
        return Ok(browser.clone());
    }
    tracing::info!("HeadlessChromeFetcher: launching headless Chrome");
    let browser = Browser::new(LaunchOptions {
        headless: true,
        ..Default::default()
    })
    .map_err(|e| {
        tracing::error!(err = %e, "HeadlessChromeFetcher: failed to launch browser");
        FetchError::BrowserLaunch
    })?;
    tracing::info!("HeadlessChromeFetcher: browser launched successfully");
    *guard = Some(browser.clone());
    Ok(browser)
}

/// Evicts the stored browser from `slot`, returning the old handle.
///
/// Used for crash recovery: the shared handle is dropped so the next
/// [`ensure_browser`] launches a fresh Chrome. Under concurrency the mutex
/// serializes eviction, so only one task performs the relaunch.
fn take_browser(slot: &Arc<Mutex<Option<Browser>>>) -> Option<Browser> {
    slot.lock().take()
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

/// Runs a single fetch against an already-obtained [`Browser`] handle.
///
/// Opens a fresh tab, navigates to `url`, waits for navigation, extracts
/// the rendered HTML via the configured extractor, captures the final URL
/// (after redirects), and closes the tab. This is the pure, browser-bound
/// half of a fetch — it performs no launch, eviction, or retry logic, so it
/// can be run inside `spawn_blocking` and re-used across attempts.
///
/// All headless_chrome failures are routed through [`classify_render_error`]
/// so a dead WebSocket surfaces as [`FetchError::BrowserCrash`] (a shared,
/// connection-level condition) rather than [`FetchError::Render`] (a
/// per-tab condition that must never evict the shared browser).
fn run_fetch_on_browser(
    browser: &Browser,
    url: &str,
    options: &FetchOptions,
    extractors: &HashMap<OutputFormat, Arc<dyn Extractor>>,
) -> Result<FetchOutput, FetchError> {
    let tab = browser
        .new_tab()
        .map_err(|e| classify_render_error(&e.to_string()))?;

    // Navigate to the URL.
    tracing::trace!(url = %url, "HeadlessChromeFetcher: navigating to URL");
    tab.navigate_to(url)
        .map_err(|e| classify_render_error(&e.to_string()))?
        .wait_until_navigated()
        .map_err(|e| classify_render_error(&e.to_string()))?;
    tracing::trace!("HeadlessChromeFetcher: navigation complete");

    // Get the rendered HTML from the tab.
    tracing::trace!("HeadlessChromeFetcher: getting page HTML");
    let html = tab
        .get_content()
        .map_err(|e| classify_render_error(&e.to_string()))?;
    tracing::debug!(
        html_len = html.len(),
        "HeadlessChromeFetcher: HTML retrieved"
    );

    // Apply extraction based on the requested format.
    tracing::trace!(format = ?options.format, "HeadlessChromeFetcher: extracting content");
    let content = match extractors.get(&options.format) {
        Some(extractor) => extractor.extract(&html),
        None => html,
    };
    tracing::debug!(
        content_len = content.len(),
        "HeadlessChromeFetcher: content extracted"
    );

    // Try to get final URL (after redirects).
    let final_url = tab.get_url();
    tracing::debug!(final_url = %final_url, "HeadlessChromeFetcher: final URL");

    // Close the tab.
    tracing::trace!("HeadlessChromeFetcher: closing tab");
    let _ = tab.close(true);

    Ok(FetchOutput {
        content,
        url: final_url,
        status: 200,
        content_type: "text/html".to_owned(),
    })
}


/// One fetch attempt against the cached browser: ensure a handle, run the
/// tab sequence.
///
/// On a connection-level death ([`FetchError::BrowserCrash`]), evicts the
/// shared handle so the next attempt relaunches. Per-tab failures
/// ([`FetchError::Render`]) are returned without eviction — evicting on them
/// would kill other sessions' in-flight tabs under concurrency.
fn fetch_once(
    browser_slot: &Arc<Mutex<Option<Browser>>>,
    url: &str,
    options: &FetchOptions,
    extractors: &HashMap<OutputFormat, Arc<dyn Extractor>>,
) -> Result<FetchOutput, FetchError> {
    let browser = ensure_browser(browser_slot)?;
    let result = run_fetch_on_browser(&browser, url, options, extractors);
    if result.is_err() {
        tracing::warn!(err = ?result.as_ref().err(), "HeadlessChromeFetcher: fetch failed");
        // Evict only on connection death, never on per-tab failures.
        if matches!(result, Err(FetchError::BrowserCrash | FetchError::BrowserLaunch)) {
            tracing::info!("HeadlessChromeFetcher: clearing browser for crash recovery");
            take_browser(browser_slot);
        }
    }
    result
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

        // First attempt.
        match fetch_once(&self.browser, url, &options, &self.extractors) {
            Err(FetchError::BrowserCrash) => {
                // Connection-level death: the shared WebSocket is gone.
                // `fetch_once` already evicted the handle; relaunch and retry exactly once.
                tracing::info!("HeadlessChromeFetcher: retrying after connection death");
                fetch_once(&self.browser, url, &options, &self.extractors)
            }
            other => other,
        }
    }

    async fn shutdown(&self) {
        tracing::info!("HeadlessChromeFetcher: shutting down");
        if let Some(browser) = take_browser(&self.browser) {
            tracing::debug!("HeadlessChromeFetcher: dropping browser (kills Chromium process)");
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
