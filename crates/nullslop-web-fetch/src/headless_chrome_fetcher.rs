//! Headless Chrome fetcher — fetches JS-rendered pages via Chromium.
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

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use headless_chrome::{Browser, LaunchOptions};

use crate::{FetchError, FetchOptions, FetchOutput, OutputFormat, WebFetcher};

/// A web fetcher that uses headless Chrome to render JavaScript-heavy pages.
///
/// The browser is lazily launched on the first `fetch()` call and reused
/// across subsequent calls. Thread-safe via `Arc<Mutex<Option<Browser>>>`.
pub struct HeadlessChromeFetcher {
    browser: Arc<Mutex<Option<Browser>>>,
}

impl HeadlessChromeFetcher {
    /// Creates a new fetcher without launching a browser.
    ///
    /// The browser will be launched on the first `fetch()` call.
    #[must_use]
    pub fn new() -> Self {
        Self {
            browser: Arc::new(Mutex::new(None)),
        }
    }

    /// Ensures a browser is running, launching one if necessary.
    fn ensure_browser(&self) -> Result<Browser, FetchError> {
        let mut guard = self.browser.lock().map_err(|_| FetchError::BrowserCrash)?;
        if let Some(ref browser) = *guard {
            return Ok(browser.clone());
        }
        let browser = Browser::new(LaunchOptions {
            headless: true,
            ..Default::default()
        })
        .map_err(|_| FetchError::BrowserLaunch)?;
        *guard = Some(browser.clone());
        Ok(browser)
    }

    /// Clears the stored browser (for crash recovery).
    fn take_browser(&self) -> Option<Browser> {
        self.browser.lock().ok().and_then(|mut guard| guard.take())
    }

    /// Extracts content from a tab based on the output format.
    fn extract_content(
        tab: &headless_chrome::Tab,
        format: OutputFormat,
    ) -> Result<String, FetchError> {
        match format {
            OutputFormat::Html => tab
                .get_content()
                .map_err(|e| FetchError::Render(e.to_string())),
            OutputFormat::Text => {
                let js = "document.body.innerText";
                let result = tab
                    .evaluate(js, false)
                    .map_err(|e| FetchError::Render(e.to_string()))?;
                let text = result
                    .value
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                Ok(text)
            }
            OutputFormat::Markdown => {
                // Get the HTML source and strip tags (same as HttpFetcher's text mode).
                let html = tab
                    .get_content()
                    .map_err(|e| FetchError::Render(e.to_string()))?;
                Ok(strip_html_tags(&html))
            }
        }
    }
}

/// Strips HTML tags from content using regex (same approach as HttpFetcher).
fn strip_html_tags(html: &str) -> String {
    use std::sync::LazyLock;
    static RE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"<[^>]*>").expect("valid regex"));

    let text = RE.replace_all(html, "");
    // Collapse whitespace and trim.
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[async_trait]
impl WebFetcher for HeadlessChromeFetcher {
    async fn fetch(&self, url: &str, options: FetchOptions) -> Result<FetchOutput, FetchError> {
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
            tab.navigate_to(url)
                .map_err(|e| FetchError::Render(e.to_string()))?
                .wait_until_navigated()
                .map_err(|e| FetchError::Render(e.to_string()))?;

            // Extract content.
            let content = Self::extract_content(&tab, options.format)?;

            // Try to get final URL (after redirects).
            let final_url = tab.get_url();

            // Close the tab.
            let _ = tab.close(true);

            Ok(FetchOutput {
                content,
                url: final_url,
                status: 200,
                content_type: "text/html".to_owned(),
            })
        })();

        // On failure, clear the browser (crash recovery).
        if result.is_err() {
            // Check if the error suggests a browser crash.
            if matches!(
                result,
                Err(FetchError::BrowserCrash | FetchError::BrowserLaunch)
            ) {
                self.take_browser();
            }
        }

        result
    }

    async fn shutdown(&self) {
        if let Some(browser) = self.take_browser() {
            // Drop the browser — this kills the Chromium process.
            drop(browser);
        }
    }
}

impl Default for HeadlessChromeFetcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[rstest::rstest]
    fn strip_html_tags_removes_tags() {
        let html = "<html><body><h1>Hello</h1><p>World</p></body></html>";
        let text = strip_html_tags(html);
        assert_eq!(text, "Hello World");
    }

    #[rstest::rstest]
    fn strip_html_tags_handles_empty() {
        let text = strip_html_tags("");
        assert!(text.is_empty());
    }

    #[rstest::rstest]
    fn strip_html_tags_collapses_whitespace() {
        let html = "<p>  lots   of   space  </p>";
        let text = strip_html_tags(html);
        assert_eq!(text, "lots of space");
    }
}
