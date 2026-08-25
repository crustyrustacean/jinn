//! Headless Chrome fetcher - fetches JS-rendered pages via Chromium.
//!
//! A thin [`WebFetcher`] adapter over [`SharedBrowser`]: it delegates page
//! rendering to the shared browser process and applies content extraction
//! afterward. The browser lifecycle (lazy launch, self-heal, retry, shutdown)
//! lives in [`crate::shared_browser`]; this module only owns the
//! fetcher-specific concern of turning rendered HTML into the requested
//! [`OutputFormat`].
//!
//! Both this fetcher and the browser-backed searcher attach to the same
//! `Arc<SharedBrowser>` for a given mode (headless or headed), so they share
//! one process and one warmed profile.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use crate::shared_browser::SharedBrowser;
use crate::{Extractor, FetchError, FetchOptions, FetchOutput, OutputFormat, WebFetcher};

/// A web fetcher that uses Chrome (headless or headed) to render
/// JavaScript-heavy pages.
///
/// Wraps a [`SharedBrowser`]. The browser is lazily launched on the first
/// `fetch()` call (by the shared browser) and reused across subsequent calls.
///
/// Content extraction is delegated to [`Extractor`] implementations looked
/// up by [`OutputFormat`]. Formats without a registered extractor (e.g.,
/// [`OutputFormat::Html`]) return the raw page HTML unchanged.
pub struct HeadlessChromeFetcher {
    /// The shared browser process this fetcher renders through.
    browser: Arc<SharedBrowser>,
    /// Extractor implementations keyed by output format.
    /// Formats not in the map (e.g., `Html`) pass through raw content.
    extractors: HashMap<OutputFormat, Arc<dyn Extractor>>,
}

impl HeadlessChromeFetcher {
    /// Creates a new fetcher over the given shared browser, without launching it.
    ///
    /// The browser is launched on the first `fetch()` call. Share the same
    /// `Arc<SharedBrowser>` across consumers that should ride one process.
    #[must_use]
    pub fn with_shared(
        browser: Arc<SharedBrowser>,
        extractors: HashMap<OutputFormat, Arc<dyn Extractor>>,
    ) -> Self {
        Self {
            browser,
            extractors,
        }
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

#[async_trait]
impl WebFetcher for HeadlessChromeFetcher {
    async fn fetch(&self, url: &str, options: FetchOptions) -> Result<FetchOutput, FetchError> {
        self.fetch_observed(url, options, std::sync::Arc::new(|_| {}))
            .await
    }

    async fn fetch_observed(
        &self,
        url: &str,
        options: FetchOptions,
        on_progress: crate::challenge::ProgressFn,
    ) -> Result<FetchOutput, FetchError> {
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
        // (render + extraction) on the blocking pool. The shared browser slot
        // is behind an Arc, so the cached Chrome is still reused across calls.
        let browser = self.browser.clone();
        let extractors = self.extractors.clone();
        let url_owned = url.to_owned();
        let join = tokio::task::spawn_blocking(move || {
            let page = browser.render_page_observed(&url_owned, &on_progress)?;
            let content = extract_content(&page.html, &options, &extractors);
            tracing::debug!(
                content_len = content.len(),
                "HeadlessChromeFetcher: content extracted"
            );
            Ok::<_, FetchError>(FetchOutput {
                content,
                url: page.final_url,
                status: 200,
                content_type: "text/html".to_owned(),
            })
        });
        // Map a panic inside the blocking task to a Render error rather than
        // propagating the JoinError; headless_chrome has panicking code paths.
        match join.await {
            Ok(inner) => inner,
            Err(_join_err) => Err(FetchError::Render("browser task panicked".to_owned())),
        }
    }

    async fn shutdown(&self) {
        self.browser.shutdown().await;
    }
}

#[cfg(test)]
mod tests;
