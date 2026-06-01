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
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use headless_chrome::{Browser, LaunchOptions};
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

    /// Ensures a browser is running, launching one if necessary.
    fn ensure_browser(&self) -> Result<Browser, FetchError> {
        let mut guard = self
            .browser
            .lock()
            .map_err(|_lock_err| FetchError::BrowserCrash)?;
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

    /// Clears the stored browser (for crash recovery).
    fn take_browser(&self) -> Option<Browser> {
        let mut guard = self.browser.lock().ok()?;
        guard.take()
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

        let browser = self.ensure_browser()?;

        // Open a new tab, navigate, extract content, close tab.
        let result = (|| -> Result<FetchOutput, FetchError> {
            let tab = browser
                .new_tab()
                .map_err(|e| FetchError::Render(e.to_string()))?;

            // Navigate to the URL.
            tracing::trace!(url = %url, "HeadlessChromeFetcher: navigating to URL");
            tab.navigate_to(url)
                .map_err(|e| FetchError::Render(e.to_string()))?
                .wait_until_navigated()
                .map_err(|e| FetchError::Render(e.to_string()))?;
            tracing::trace!("HeadlessChromeFetcher: navigation complete");

            // Get the rendered HTML from the tab.
            tracing::trace!("HeadlessChromeFetcher: getting page HTML");
            let html = tab
                .get_content()
                .map_err(|e| FetchError::Render(e.to_string()))?;
            tracing::debug!(
                html_len = html.len(),
                "HeadlessChromeFetcher: HTML retrieved"
            );

            // Apply extraction based on the requested format.
            tracing::trace!(format = ?options.format, "HeadlessChromeFetcher: extracting content");
            let content = match self.extractors.get(&options.format) {
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
        })();

        // On failure, clear the browser (crash recovery).
        if let Err(err) = &result {
            tracing::warn!(err = ?err, "HeadlessChromeFetcher: fetch failed");
            // Check if the error suggests a browser crash.
            if matches!(
                result,
                Err(FetchError::BrowserCrash | FetchError::BrowserLaunch)
            ) {
                tracing::info!("HeadlessChromeFetcher: clearing browser for crash recovery");
                self.take_browser();
            }
        }

        result
    }

    async fn shutdown(&self) {
        tracing::info!("HeadlessChromeFetcher: shutting down");
        if let Some(browser) = self.take_browser() {
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
